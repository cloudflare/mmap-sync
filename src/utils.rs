use std::fs::File;
use std::io;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;

/// Set the length of the file to the specified length.
pub(crate) fn set_len(file: &File, len: i64) -> Result<(), io::Error> {
    // On Linux platforms, `.set_len()` may not return an error of the disk is full, so we allocate
    // the entire file to ensure space is available.
    #[cfg(target_os = "linux")]
    match unsafe { libc::posix_fallocate(file.as_raw_fd(), 0, len) } {
        0 => Ok(()),
        err => Err(io::Error::from_raw_os_error(err)),
    }
    // Support for non-Linux platforms is best-effort.
    #[cfg(not(target_os = "linux"))]
    file.set_len(len).map_err(FailedStateRead)
}
