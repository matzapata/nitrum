//! Low-level read/write helpers for raw Unix file descriptors.

/// Reads exactly `buf.len()` bytes from `fd`, returning early on EOF or error.
pub fn read_exact_raw(fd: libc::c_int, buf: &mut [u8]) -> std::io::Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = unsafe {
            libc::read(
                fd,
                buf[pos..].as_mut_ptr().cast::<libc::c_void>(),
                buf.len() - pos,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        pos += usize::try_from(n).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative read size encountered",
            )
        })?;
    }
    Ok(())
}

/// Writes all bytes in `buf` to `fd`, retrying partial writes until complete.
pub fn write_all_raw(fd: libc::c_int, buf: &[u8]) -> std::io::Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = unsafe {
            libc::write(
                fd,
                buf[pos..].as_ptr().cast::<libc::c_void>(),
                buf.len() - pos,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        pos += usize::try_from(n).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative write size encountered",
            )
        })?;
    }
    Ok(())
}

/// Writes all bytes from `bufs` to `fd`, coalescing slices into one `writev` per attempt.
pub fn write_all_vectored_raw(fd: libc::c_int, bufs: &[&[u8]]) -> std::io::Result<()> {
    let mut head = 0usize;
    let mut offset = 0usize;

    while head < bufs.len() {
        let iovecs: Vec<libc::iovec> = bufs[head..]
            .iter()
            .enumerate()
            .filter_map(|(i, slice)| {
                let slice_offset = if i == 0 { offset } else { 0 };
                if slice_offset >= slice.len() {
                    return None;
                }
                Some(libc::iovec {
                    iov_base: slice[slice_offset..].as_ptr().cast_mut().cast(),
                    iov_len: slice.len() - slice_offset,
                })
            })
            .collect();

        if iovecs.is_empty() {
            break;
        }

        let n = unsafe {
            libc::writev(
                fd,
                iovecs.as_ptr(),
                libc::c_int::try_from(iovecs.len()).map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "too many iovecs for writev",
                    )
                })?,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "writev returned 0",
            ));
        }

        let mut remaining = usize::try_from(n).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative write size encountered",
            )
        })?;

        while remaining > 0 {
            let slice_len = bufs[head].len() - offset;
            if remaining < slice_len {
                offset += remaining;
                remaining = 0;
            } else {
                remaining -= slice_len;
                head += 1;
                offset = 0;
            }
        }
    }

    Ok(())
}
