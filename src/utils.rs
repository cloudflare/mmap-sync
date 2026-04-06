use std::fs::File;
use std::io;
#[cfg(unix)]
use std::os::fd::AsRawFd;

/// Set the length of the file to the specified length.
pub(crate) fn set_len(file: &File, len: i64) -> Result<(), io::Error> {
    // On Unix platforms, `.set_len()` may not return an error of the disk is full, so we allocate
    // the entire file to ensure space is available.
    #[cfg(unix)]
    match unsafe { libc::posix_fallocate(file.as_raw_fd(), 0, len) } {
        0 => Ok(()),
        err => Err(io::Error::from_raw_os_error(err)),
    }
    // Support for non-Linux platforms is best-effort.
    #[cfg(not(unix))]
    data_file.set_len(STATE_SIZE).map_err(FailedStateRead)
}
