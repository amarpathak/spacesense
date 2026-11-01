//! readdir + fstatat backend: one `fstatat` per entry. Portable baseline.

use super::{Entry, EntryError, Kind};
use std::ffi::CStr;
use std::io;

/// List the open directory `fd` (not closed; it is dup'ed for `fdopendir`).
pub fn read_dir(fd: libc::c_int, f: &mut dyn FnMut(Entry<'_>)) -> io::Result<Vec<EntryError>> {
    let fd = unsafe { libc::dup(fd) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let dp = unsafe { libc::fdopendir(fd) };
    if dp.is_null() {
        let e = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    let mut errors = Vec::new();
    let result = loop {
        // readdir signals errors only through errno, so clear it first.
        unsafe { *libc::__error() = 0 };
        let ent = unsafe { libc::readdir(dp) };
        if ent.is_null() {
            let errno = unsafe { *libc::__error() };
            break if errno == 0 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(errno))
            };
        }
        let name_c = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
        let name = name_c.to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::fstatat(
                libc::dirfd(dp),
                name_c.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if rc != 0 {
            errors.push((
                name.to_vec(),
                io::Error::last_os_error().raw_os_error().unwrap_or(0),
            ));
            continue;
        }
        let kind = match st.st_mode & libc::S_IFMT {
            libc::S_IFREG => Kind::File,
            libc::S_IFDIR => Kind::Dir,
            libc::S_IFLNK => Kind::Symlink,
            _ => Kind::Other,
        };
        f(Entry {
            name,
            kind,
            dev: st.st_dev as i64,
            ino: st.st_ino,
            nlink: st.st_nlink as u32,
            alloc: st.st_blocks as u64 * 512,
            mtime: st.st_mtime,
            // A mounted directory reports the mounted device; the walker compares devices.
            mount_point: false,
            dataless: st.st_flags & super::SF_DATALESS != 0,
        });
    };
    unsafe { libc::closedir(dp) };
    result.map(|_| errors)
}
