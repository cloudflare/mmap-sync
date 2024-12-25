//! The `guard` module manages read access to shared memory, utilizing RAII (Resource Acquisition
//! Is Initialization) principles to control access lifecycle.
//!
//! `ReadGuard` is the primary structure in this module. On creation, `ReadGuard` provides a
//! reference to an entity in shared memory. This reference can be safely used until the
//! `grace_duration` set in the `write` method of the `synchronizer` module expires.
//!
//! `ReadGuard` relinquishes memory access rights automatically when it goes out of scope and
//! gets dropped, thus avoiding potential memory corruption due to lingering references.
//! Therefore, users must ensure all `ReadGuard` instances are dropped before `grace_duration`
//! expires.
//!
//! The `synchronizer` module utilizes this `guard` module to manage memory safety, allowing
//! users to focus on their application logic.
use rkyv::{Archive, Archived};
use std::ops::Deref;

use crate::instance::InstanceVersion;
use crate::state::State;

/// An RAII implementation of a “scoped read lock” of a `State`
pub(crate) struct ReadGuard<'a> {
    state: &'a mut State,
    version: InstanceVersion,
}

impl<'a> ReadGuard<'a> {
    /// Creates new `ReadGuard` with specified parameters
    pub(crate) fn new(state: &'a mut State, version: InstanceVersion) -> Self {
        state.rlock(version);
        ReadGuard { version, state }
    }
}

impl Drop for ReadGuard<'_> {
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
    pub(crate) fn new(guard: ReadGuard<'a>, entity: &'a Archived<T>, switched: bool) -> Self {
        ReadResult {
            _guard: guard,
            entity,
            switched,
        }
    }

    /// Indicates whether data was switched during last read
    pub fn is_switched(&self) -> bool {
        self.switched
    }
}

impl<T: Archive> Deref for ReadResult<'_, T> {
    type Target = Archived<T>;

    /// Dereferences stored `entity` for easier access
    fn deref(&self) -> &Archived<T> {
        self.entity
    }
}
