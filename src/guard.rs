//! The `guard` module provides a safe interface for accessing shared memory.
//!
//! It includes functionality for managing read and write access to the memory-mapped files, ensuring safety of data operations. It is leveraged by the `synchronizer` module to perform concurrent read/write operations.

use rkyv::{Archive, Archived};
use std::ops::Deref;

use crate::instance::InstanceVersion;
use crate::state::State;
use crate::synchronizer::SynchronizerError;

/// An RAII implementation of a “scoped read lock” of a `State`
pub(crate) struct ReadGuard<'a> {
    state: &'a mut State,
    version: InstanceVersion,
}

impl<'a> ReadGuard<'a> {
    /// Creates new `ReadGuard` with specified parameters
    pub(crate) fn new(
        state: &'a mut State,
        version: InstanceVersion,
    ) -> Result<Self, SynchronizerError> {
        state.rlock(version);
        Ok(ReadGuard { version, state })
    }
}

impl<'a> Drop for ReadGuard<'a> {
    /// Unlocks stored `version` when `ReadGuard` goes out of scope
    fn drop(&mut self) {
        self.state.runlock(self.version);
    }
}

/// `Synchronizer` result
pub struct ReadResult<'a, T: Archive> {
    _guard: ReadGuard<'a>,
    entity: &'a Archived<T>,
    switched: bool,
}

impl<'a, T: Archive> ReadResult<'a, T> {
    /// Creates new `ReadResult` with specified parameters
    pub(crate) fn new(_guard: ReadGuard<'a>, entity: &'a Archived<T>, switched: bool) -> Self {
        ReadResult {
            _guard,
            entity,
            switched,
        }
    }

    /// Indicates whether data was switched during last read
    pub fn is_switched(&self) -> bool {
        self.switched
    }
}

impl<'a, T: Archive> Deref for ReadResult<'a, T> {
    type Target = Archived<T>;

    /// Dereferences stored `entity` for easier access
    fn deref(&self) -> &Archived<T> {
        self.entity
    }
}
