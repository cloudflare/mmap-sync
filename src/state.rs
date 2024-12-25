use memmap2::MmapMut;
use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::ops::{Add, DerefMut};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{mem, thread};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::instance::InstanceVersion;
use crate::locks::WriteLockStrategy;
use crate::synchronizer::SynchronizerError;
use crate::synchronizer::SynchronizerError::*;

const STATE_SIZE: usize = mem::size_of::<State>();

/// State stored in memory for synchronization using atomics
#[repr(C)]
pub(crate) struct State<const SD: usize = 1_000_000_000> {
    /// Current data instance version
    version: AtomicU64,
    /// Current number of readers for each data instance
    idx_readers: [AtomicU32; 2],
}

impl State {
    /// Initialize new state with zero values
    pub(crate) fn new() -> State {
        State {
            version: AtomicU64::new(0),
            idx_readers: [AtomicU32::new(0), AtomicU32::new(0)],
        }
    }

    /// Return state's current instance version
    #[inline]
    pub(crate) fn version(&self) -> Result<InstanceVersion, SynchronizerError> {
        self.version.load(Ordering::SeqCst).try_into()
    }

    /// Locks given `version` of the state for reading
    #[inline]
    pub(crate) fn rlock(&mut self, version: InstanceVersion) {
        self.idx_readers[version.idx()].fetch_add(1, Ordering::SeqCst);
    }

    /// Acquire next `idx` of the state for writing
    #[inline]
    pub(crate) fn acquire_next_idx(
        &self,
        grace_duration: Duration,
        sleep_duration: Duration,
    ) -> (usize, bool) {
        // calculate `next_idx` to acquire, in case of uninitialized version use 0
        let next_idx = match InstanceVersion::try_from(self.version.load(Ordering::SeqCst)) {
            Ok(version) => (version.idx() + 1) % 2,
            Err(_) => 0,
        };

        // check number of readers using `next_idx`
        let num_readers = &self.idx_readers[next_idx];

        // wait until either no more readers left for `next_idx` or grace period has expired
        let grace_expiring_at = Instant::now().add(grace_duration);
        let mut reset = false;
        while num_readers.load(Ordering::SeqCst) > 0 {
            // we should reach here only when one of the readers dies without decrement
            if Instant::now().gt(&grace_expiring_at) {
                // reset number of readers after expired grace period
                num_readers.store(0, Ordering::SeqCst);
                reset = true;
                break;
            } else {
                thread::sleep(sleep_duration);
            }
        }

        (next_idx, reset)
    }

    /// Unlocks given `version` from reading
    #[inline]
    pub(crate) fn runlock(&mut self, version: InstanceVersion) {
        self.idx_readers[version.idx()].fetch_sub(1, Ordering::SeqCst);
    }

    /// Switch state to given `version`
    #[inline]
    pub(crate) fn switch_version(&mut self, version: InstanceVersion) {
        // actually change current data file index in memory mapped state
        // so new readers can switch to it when calling `read`
        self.version.swap(version.into(), Ordering::SeqCst);
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// State container stores memory mapped state file, which is used for
/// synchronization purposes with a help of atomics
pub(crate) struct StateContainer<WL> {
    /// State file path
    state_path: OsString,
    /// Modifiable memory mapped file storing state.
    ///
    /// The [`MmapMut`] type is wrapped in a [`WriteLockStrategy`] to require lock acquisition
    /// prior to writing.
    mmap: Option<WL>,
}

const STATE_SUFFIX: &str = "_state";

impl<'a, WL: WriteLockStrategy<'a>> StateContainer<WL> {
    /// Create new instance of `StateContainer`
    pub(crate) fn new(path_prefix: &OsStr) -> Self {
        let mut state_path = path_prefix.to_os_string();
        state_path.push(STATE_SUFFIX);
        StateContainer {
            state_path,
            mmap: None,
        }
    }

    /// Fetch state from existing memory mapped file or create new one.
    ///
    /// If this is a write, call the configured write lock strategy and return a lock conflict
    /// error if the lock cannot be acquired.
    #[inline]
    pub(crate) fn state<const WRITE: bool>(
        &'a mut self,
        create: bool,
    ) -> Result<&'a mut State, SynchronizerError> {
        if self.mmap.is_none() {
            self.prepare_mmap(create)?;
        }

        if WRITE {
            let mut guard = self.mmap.as_mut().unwrap().lock()?;
            Ok(unsafe { &mut *(guard.deref_mut().as_ptr() as *mut State) })
        } else {
            let mmap = self.mmap.as_ref().unwrap().read();
            Ok(unsafe { &mut *(mmap.as_ptr() as *mut State) })
        }
    }

    /// Initialize mmaped memory from the state file.
    #[inline]
    pub(crate) fn prepare_mmap(&mut self, create: bool) -> Result<(), SynchronizerError> {
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(create);

        // Only add mode on Unix-based systems to allow read/write from owner/group only
        #[cfg(unix)]
        opts.mode(0o660);

        let state_file = opts.open(&self.state_path).map_err(FailedStateRead)?;

        let mut need_init = false;
        // Reset state file size to match exactly `STATE_SIZE`
        if state_file.metadata().map_err(FailedStateRead)?.len() != STATE_SIZE as u64 {
            state_file
                .set_len(STATE_SIZE as u64)
                .map_err(FailedStateRead)?;
            need_init = true;
        }

        let mut mmap = unsafe { MmapMut::map_mut(&state_file).map_err(FailedStateRead)? };
        if need_init {
            // Create new state and write it to mapped memory
            let new_state = State::default();
            unsafe {
                mmap.as_mut_ptr()
                    .copy_from((&new_state as *const State) as *const u8, STATE_SIZE)
            };
        };

        self.mmap = Some(WL::new(mmap, state_file));
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::locks::SingleWriter;
    use crate::synchronizer::SynchronizerError;

    #[test]
    fn single_writer_lock_mode_prevents_duplicate_writer() {
        static PATH: &str = "/tmp/single_writer_lock_test";
        let mut state1 = StateContainer::<SingleWriter>::new(PATH.as_ref());
        let mut state2 = StateContainer::<SingleWriter>::new(PATH.as_ref());

        assert!(state1.state::<true>(true).is_ok());
        assert!(matches!(
            state2.state::<true>(true),
            Err(SynchronizerError::WriteLockConflict)
        ));
    }

    #[test]
    fn single_writer_lock_freed_on_drop() {
        static PATH: &str = "/tmp/single_writer_lock_drop_test";
        let mut state1 = StateContainer::<SingleWriter>::new(PATH.as_ref());
        let mut state2 = StateContainer::<SingleWriter>::new(PATH.as_ref());

        assert!(state1.state::<true>(true).is_ok());
        drop(state1);
        assert!(state2.state::<true>(true).is_ok());
    }
}
