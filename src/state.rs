use memmap2::MmapMut;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::ops::Add;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{mem, thread};

#[cfg(all(unix, feature = "write-lock"))]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::instance::InstanceVersion;
use crate::synchronizer::SynchronizerError::*;
use crate::synchronizer::{SynchronizerError, WriteLockMode};

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
pub(crate) struct StateContainer {
    /// State file path
    state_path: OsString,
    /// Modifiable memory mapped file storing state.
    ///
    /// We also hold the file backing the mmaped memory; this keeps the file descriptor open and
    /// preserves locks created with flock. Locks are released when dropped.
    mmap: Option<(MmapMut, File)>,
    /// Indicates if we have an exclusive write lock
    #[cfg(all(unix, feature = "write-lock"))]
    write_lock_held: bool,
}

const STATE_SUFFIX: &str = "_state";

impl StateContainer {
    /// Create new instance of `StateContainer`
    pub(crate) fn new(path_prefix: &OsStr) -> Self {
        let mut state_path = path_prefix.to_os_string();
        state_path.push(STATE_SUFFIX);
        StateContainer {
            state_path,
            mmap: None,
            #[cfg(all(unix, feature = "write-lock"))]
            write_lock_held: false,
        }
    }

    /// Fetch state from existing memory mapped file or create new one.
    #[inline]
    pub(crate) fn state_read(&mut self, create: bool) -> Result<&mut State, SynchronizerError> {
        if self.mmap.is_none() {
            self.prepare_mmap(create)?;
        }
        Ok(unsafe { &mut *(self.mmap.as_ref().unwrap().0.as_ptr() as *mut State) })
    }

    /// Fetch state from existing memory mapped file or create new one.
    ///
    /// If write locking is enabled, acquire the lock before initializing memory, returning a
    /// lock conflict error if the lock cannot be acquired.
    #[inline]
    pub(crate) fn state_write(
        &mut self,
        create: bool,
        #[allow(unused_variables)] write_lock: WriteLockMode,
    ) -> Result<&mut State, SynchronizerError> {
        if self.mmap.is_none() {
            self.prepare_mmap(create)?;
        }

        #[allow(unused_variables)]
        let (mmap, file) = self.mmap.as_mut().unwrap();

        // Acquire an exclusive write lock if `write_lock` requires it.
        #[cfg(all(unix, feature = "write-lock"))]
        if !self.write_lock_held {
            match write_lock {
                WriteLockMode::Disabled => {}
                WriteLockMode::SingleWriter => {
                    match unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } {
                        0 => self.write_lock_held = true,
                        _ => {
                            dbg!(std::io::Error::last_os_error());
                            return Err(SynchronizerError::WriteLockConflict);
                        }
                    }
                }
            }
        }

        Ok(unsafe { &mut *(mmap.as_ptr() as *mut State) })
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
        if state_file.metadata().map_err(FailedStateRead)?.len() as usize != STATE_SIZE {
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

        self.mmap = Some((mmap, state_file));
        Ok(())
    }
}

#[cfg(all(test, unix, feature = "write-lock"))]
mod tests {
    use super::*;
    use crate::synchronizer::{SynchronizerError, WriteLockMode};

    #[test]
    fn single_writer_lock_mode_prevents_duplicate_writer() {
        static PATH: &str = "/tmp/single_writer_lock_test";
        let mut state1 = StateContainer::new(PATH.as_ref());
        let mut state2 = StateContainer::new(PATH.as_ref());

        assert!(state1
            .state_write(true, WriteLockMode::SingleWriter)
            .is_ok());
        assert!(matches!(
            state2.state_write(true, WriteLockMode::SingleWriter),
            Err(SynchronizerError::WriteLockConflict)
        ));
    }

    #[test]
    fn single_writer_lock_freed_on_drop() {
        static PATH: &str = "/tmp/single_writer_lock_drop_test";
        let mut state1 = StateContainer::new(PATH.as_ref());
        let mut state2 = StateContainer::new(PATH.as_ref());

        assert!(state1
            .state_write(true, WriteLockMode::SingleWriter)
            .is_ok());
        drop(state1);
        assert!(state2
            .state_write(true, WriteLockMode::SingleWriter)
            .is_ok());
    }
}
