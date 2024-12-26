//! Lock strategies.
//!
//! # Safety
//! This crate requires that only one writer be active at a time. It is the caller's responsibility
//! to uphold th is guarantee.
//!
//! Note: if multiple writers are active, it is the caller's responsibility to ensure that each
//! writer is configured with an appropriate lock strategy. For example, mixing the `LockDisabled`
//! strategy with any other strategy is incorrect because it disables lock checks from one of the
//! synchronizers.

#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::{
    fs::File,
    ops::{Deref, DerefMut},
};

use memmap2::MmapMut;

use crate::synchronizer::SynchronizerError;

/// The write lock strategy supports different lock implementations which can be chosen based on
/// the guarantees required, platform support, and performance constraints.
///
/// Note: the lock implementations are sealed to avoid committing to a specific lock interface.
#[allow(private_bounds)]
pub trait WriteLockStrategy<'a>: WriteLockStrategySealed<'a> {}

/// Sealed trait i
pub(crate) trait WriteLockStrategySealed<'a> {
    type Guard: DerefMut<Target = MmapMut> + 'a;

    /// Create a new instance of this lock strategy.
    ///
    /// The `mmap` parameter will have write access controlled by the lock.
    ///
    /// The `file` parameter is required because lock strategies depending on `flock` must hold
    /// the file descriptor open so the kernel does not release the lock.
    fn new(mmap: MmapMut, file: File) -> Self;

    /// Provide read access to mmaped memory.
    fn read(&'a self) -> &'a [u8];

    /// Acquire the lock as specified by the lock strategy.
    ///
    /// On success, return a lock guard which can be used to access the underlying mmaped memory
    /// via [`Deref`]/[`DerefMut`].
    fn lock(&'a mut self) -> Result<Self::Guard, SynchronizerError>;
}

/// Lock protection is disabled.
///
/// # Safety
/// Callers must ensure that there is only a single active writer. For example, the caller might
/// ensure that only one process attempts to write to the synchronizer, and ensure that multiple
/// instances of the process are not spawned.
pub struct LockDisabled(MmapMut);

impl<'a> WriteLockStrategySealed<'a> for LockDisabled {
    type Guard = DisabledGuard<'a>;

    #[inline]
    fn new(mmap: MmapMut, _file: File) -> Self {
        // No need to hold the file descriptor because lock functionality is disabled.
        Self(mmap)
    }

    #[inline]
    fn read(&'a self) -> &'a [u8] {
        &self.0
    }

    #[inline]
    fn lock(&'a mut self) -> Result<Self::Guard, SynchronizerError> {
        Ok(DisabledGuard(&mut self.0))
    }
}

impl WriteLockStrategy<'_> for LockDisabled {}

pub struct DisabledGuard<'a>(&'a mut MmapMut);

impl Deref for DisabledGuard<'_> {
    type Target = MmapMut;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for DisabledGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

/// Acquire the lock. Once acquired, hold the lock until dropped.
///
/// The `flock` API holds the lock as long as the file descriptor is open, and closes the lock
/// when the descriptor is closed. The descriptor is automatically closed when `File` is dropped.
#[cfg(unix)]
pub struct SingleWriter {
    mmap: MmapMut,
    file: File,
    locked: bool,
}

#[cfg(unix)]
impl<'a> WriteLockStrategySealed<'a> for SingleWriter {
    type Guard = SingleWriterGuard<'a>;

    #[inline]
    fn new(mmap: MmapMut, file: File) -> Self {
        Self {
            mmap,
            file,
            locked: false,
        }
    }

    #[inline]
    fn read(&'a self) -> &'a [u8] {
        &self.mmap
    }

    #[inline]
    fn lock(&'a mut self) -> Result<Self::Guard, SynchronizerError> {
        // We already hold the lock, so return success.
        if self.locked {
            return Ok(SingleWriterGuard(&mut self.mmap));
        }

        // Acquire the lock for the first time.
        // Note: the file descriptor must remain open to hold the lock.
        match unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } {
            0 => {
                // Hold the lock until this structure is dropped.
                self.locked = true;
                Ok(SingleWriterGuard(&mut self.mmap))
            }
            _ => Err(SynchronizerError::WriteLockConflict),
        }
    }
}

#[cfg(unix)]
impl WriteLockStrategy<'_> for SingleWriter {}

/// A simple guard which does not release the lock upon being dropped.
#[cfg(unix)]
pub struct SingleWriterGuard<'a>(&'a mut MmapMut);

#[cfg(unix)]
impl Deref for SingleWriterGuard<'_> {
    type Target = MmapMut;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

#[cfg(unix)]
impl DerefMut for SingleWriterGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}
