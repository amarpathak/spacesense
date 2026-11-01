//! Directory listing backends.
//!
//! A backend lists one directory and reports, for every entry, the facts the
//! size accounting needs (type, device, inode, link count, allocated bytes,
//! mtime). It never recurses; the walker does that.

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

pub mod bulk;
pub mod stat;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// `getattrlistbulk(2)`: one syscall per batch of entries (macOS only).
    Bulk,
    /// `readdir(3)` + `fstatat(2)` per entry. Portable fallback.
    Stat,
}

impl std::str::FromStr for Backend {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "bulk" => Ok(Backend::Bulk),
            "stat" => Ok(Backend::Stat),
            _ => Err(format!("unknown backend {s:?} (expected bulk or stat)")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Other,
}

/// One directory entry. `name` borrows the backend's buffer.
#[derive(Debug)]
pub struct Entry<'a> {
    pub name: &'a [u8],
    pub kind: Kind,
    pub dev: i64,
    pub ino: u64,
    pub nlink: u32,
    /// Allocated bytes on disk (what `du` counts), not logical length.
    pub alloc: u64,
    /// Seconds since the epoch.
    pub mtime: i64,
    /// True when the entry is a mount point (a different file system is mounted on it).
    pub mount_point: bool,
    /// Cloud-only placeholder (iCloud Drive, File Provider): its content isn't
    /// on this Mac. Opening it would download it, so the walker never does.
    pub dataless: bool,
}

/// `SF_DATALESS` from <sys/stat.h>.
pub const SF_DATALESS: u32 = 0x4000_0000;

/// Per-entry failures inside an otherwise readable directory.
pub type EntryError = (Vec<u8>, i32);

impl Backend {
    /// List the open directory `fd`, calling `f` for every entry except `.` and `..`.
    /// Returns per-entry errors (name, errno); a failure to open or read the
    /// directory itself is returned as `Err`.
    pub fn read_dir(
        self,
        fd: libc::c_int,
        f: &mut dyn FnMut(Entry<'_>),
    ) -> io::Result<Vec<EntryError>> {
        match self {
            Backend::Bulk => match bulk::read_dir(fd, f) {
                // Some file systems (SMB, FAT, ...) don't support the call.
                Err(e) if e.raw_os_error() == Some(libc::ENOTSUP) => stat::read_dir(fd, f),
                r => r,
            },
            Backend::Stat => stat::read_dir(fd, f),
        }
    }
}

/// Open directory `name` relative to the open directory `parent` (or the cwd
/// when `parent` is `None`), without following a symlink in the last component.
///
/// Opening relative to a held parent fd keeps the ancestors' vnodes referenced,
/// so the kernel never re-resolves a full path. That matters when the tree has
/// more entries than `kern.maxvnodes` and the vnode cache is being recycled.
pub fn open_dir_at(parent: Option<BorrowedFd<'_>>, name: &CStr) -> io::Result<OwnedFd> {
    let base = parent.map(|p| p.as_raw_fd()).unwrap_or(libc::AT_FDCWD);
    let fd = unsafe {
        libc::openat(
            base,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}
