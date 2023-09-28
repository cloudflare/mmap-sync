use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::ops::Add;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{mem, thread};

use crate::instance::InstanceVersion;
use crate::synchronizer::SynchronizerError;
use crate::synchronizer::SynchronizerError::*;

const STATE_SIZE: usize = mem::size_of::<State>();
const SLEEP_DURATION: Duration = Duration::from_secs(1);

/// State stored in memory for synchronization using atomics
#[repr(C)]
pub(crate) struct State {
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
    pub(crate) fn acquire_next_idx(&self, grace_duration: Duration) -> (usize, bool) {
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
                thread::sleep(SLEEP_DURATION);
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
    state_path: String,
    /// Modifiable memory mapped file storing state
    mmap: Option<MmapMut>,
}

const STATE_SUFFIX: &str = "_state";

impl StateContainer {
    /// Create new instance of `StateContainer`
    pub(crate) fn new(path: &str) -> Self {
        let state_path = path.to_owned() + STATE_SUFFIX;
        StateContainer {
            state_path,
            mmap: None,
        }
    }

    /// Fetch state from existing memory mapped file or create new one
    #[inline]
    pub(crate) fn state(&mut self, create: bool) -> Result<&mut State, SynchronizerError> {
        if self.mmap.is_none() {
            let state_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(create)
                .mode(0o660) // set file mode to allow read/write from owner/group only
                .open(&self.state_path)
                .map_err(FailedStateRead)?;

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

            self.mmap = Some(mmap);
        }
        Ok(unsafe { &mut *(self.mmap.as_ref().unwrap().as_ptr() as *mut State) })
    }
}
