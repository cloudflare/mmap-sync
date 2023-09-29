use std::ffi::{OsStr, OsString};

use crate::synchronizer::SynchronizerError;
use crate::synchronizer::SynchronizerError::*;

/// `InstanceVersion` represents data instance and consists of the following components:
/// - data idx (0 or 1)   - 1 bit
/// - data size (<549 GB) - 39 bits
/// - data checksum       - 24 bits
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceVersion(pub(crate) u64);

const DATA_SIZE_BITS: usize = 39;
const DATA_CHECKSUM_BITS: usize = 24;

impl InstanceVersion {
    /// Create new `InstanceVersion` from data instance `idx`, `size` and `checksum`
    #[inline]
    pub(crate) fn new(
        idx: usize,
        size: usize,
        checksum: u64,
    ) -> Result<InstanceVersion, SynchronizerError> {
        let mut res: u64 = 0;

        if idx > 1 || size >= 1 << DATA_SIZE_BITS {
            return Err(InvalidInstanceVersionParams);
        }

        res |= (idx as u64) & 1;
        res |= ((size as u64) & ((1 << DATA_SIZE_BITS) - 1)) << 1;
        res |= (checksum & ((1 << DATA_CHECKSUM_BITS) - 1)) << (DATA_SIZE_BITS + 1);

        Ok(InstanceVersion(res))
    }

    /// Get data instance `idx` (0 or 1)
    #[inline]
    pub(crate) fn idx(&self) -> usize {
        self.0 as usize & 1
    }

    /// Get data instance `size`
    #[inline]
    pub(crate) fn size(&self) -> usize {
        (self.0 as usize >> 1) & ((1 << DATA_SIZE_BITS) - 1)
    }

    /// Get data instance `checksum`
    #[cfg(test)]
    pub(crate) fn checksum(&self) -> u64 {
        self.0 >> (DATA_SIZE_BITS + 1)
    }

    /// Get data instance `path`
    #[inline]
    pub(crate) fn path(&self, path_prefix: &OsStr) -> OsString {
        let mut path = path_prefix.to_os_string();
        path.push(format!("_data_{}", self.idx()));
        path
    }
}

impl TryFrom<u64> for InstanceVersion {
    type Error = SynchronizerError;

    /// Convert from `u64` to `InstanceVersion`
    #[inline]
    fn try_from(v: u64) -> Result<InstanceVersion, Self::Error> {
        if v == 0 {
            Err(UninitializedState)
        } else {
            Ok(InstanceVersion(v))
        }
    }
}

impl From<InstanceVersion> for u64 {
    /// Convert from `InstanceVersion` to `u64`
    #[inline]
    fn from(v: InstanceVersion) -> Self {
        v.0
    }
}

#[cfg(test)]
mod tests {
    use crate::instance::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_new(idx in 0..2usize, size in 0..1usize << DATA_SIZE_BITS, checksum in 0..u64::MAX) {
            let v = InstanceVersion::new(idx, size, checksum).unwrap();

            assert_eq!(idx, v.idx());
            assert_eq!(size, v.size());
            assert_eq!(checksum & ((1 << DATA_CHECKSUM_BITS) - 1), v.checksum());
        }
    }
}
