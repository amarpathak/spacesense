//! `getattrlistbulk(2)` backend (macOS).
//!
//! The kernel fills a buffer with one packed record per entry. Layout of a
//! record (see getattrlist(2) / getattrlistbulk(2)):
//!
//! ```text
//! u32               record length
//! attribute_set_t   returned attrs (ATTR_CMN_RETURNED_ATTRS, always first)
//! u32               ATTR_CMN_ERROR (placed right after the returned set)
//! common attrs in bit order: NAME, DEVID, OBJTYPE, MODTIME, FLAGS, FILEID
//! dir attrs in bit order:    MOUNTSTATUS, ALLOCSIZE      (directories only)
//! file attrs in bit order:   LINKCOUNT, ALLOCSIZE        (files only)
//! ```
//!
//! Only attributes whose bit is set in the returned set are present, so we
//! walk the record checking each bit. Values are 4-byte aligned, so 8-byte
//! values are read unaligned.

use super::{Entry, EntryError, Kind};
use libc::{
    attribute_set_t, attrlist, attrreference_t, ATTR_BIT_MAP_COUNT, ATTR_CMN_DEVID,
    ATTR_CMN_FILEID, ATTR_CMN_FLAGS, ATTR_CMN_MODTIME, ATTR_CMN_NAME, ATTR_CMN_OBJTYPE, ATTR_CMN_RETURNED_ATTRS,
    ATTR_DIR_ALLOCSIZE, ATTR_DIR_MOUNTSTATUS, ATTR_FILE_ALLOCSIZE, ATTR_FILE_LINKCOUNT,
    DIR_MNTSTATUS_MNTPOINT,
};
use std::cell::RefCell;
use std::io;

/// Not exported by the libc crate. <sys/attr.h>: ATTR_CMN_ERROR 0x20000000.
const ATTR_CMN_ERROR: u32 = 0x2000_0000;

const BUF_SIZE: usize = 256 * 1024;

// fsobj_type_t values from <sys/vnode.h>.
const VREG: u32 = 1;
const VDIR: u32 = 2;
const VLNK: u32 = 5;

thread_local! {
    static BUF: RefCell<Vec<u8>> = RefCell::new(vec![0u8; BUF_SIZE]);
}

fn attr_list() -> attrlist {
    attrlist {
        bitmapcount: ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: ATTR_CMN_RETURNED_ATTRS
            | ATTR_CMN_ERROR
            | ATTR_CMN_NAME
            | ATTR_CMN_DEVID
            | ATTR_CMN_OBJTYPE
            | ATTR_CMN_MODTIME
            | ATTR_CMN_FLAGS
            | ATTR_CMN_FILEID,
        volattr: 0,
        dirattr: ATTR_DIR_MOUNTSTATUS | ATTR_DIR_ALLOCSIZE,
        fileattr: ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE,
        forkattr: 0,
    }
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take<T: Copy>(&mut self) -> Option<T> {
        let size = std::mem::size_of::<T>();
        let end = self.pos.checked_add(size)?;
        if end > self.buf.len() {
            return None;
        }
        let v = unsafe { std::ptr::read_unaligned(self.buf.as_ptr().add(self.pos) as *const T) };
        self.pos = end;
        Some(v)
    }
}

/// List the open directory `fd` (not closed; its read position is consumed).
pub fn read_dir(fd: libc::c_int, f: &mut dyn FnMut(Entry<'_>)) -> io::Result<Vec<EntryError>> {
    BUF.with(|b| read_fd(fd, &mut b.borrow_mut(), f))
}

fn read_fd(
    fd: libc::c_int,
    buf: &mut [u8],
    f: &mut dyn FnMut(Entry<'_>),
) -> io::Result<Vec<EntryError>> {
    let mut al = attr_list();
    let mut errors = Vec::new();
    loop {
        let n = unsafe {
            libc::getattrlistbulk(
                fd,
                &mut al as *mut attrlist as *mut libc::c_void,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Ok(errors);
        }
        let mut off = 0usize;
        for _ in 0..n {
            let rec_len = match buf.get(off..off + 4) {
                Some(b) => u32::from_ne_bytes(b.try_into().unwrap()) as usize,
                None => return Err(io::Error::new(io::ErrorKind::InvalidData, "short buffer")),
            };
            if rec_len < 4 || off + rec_len > buf.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bad record length",
                ));
            }
            let rec = &buf[off..off + rec_len];
            parse_record(rec, f, &mut errors);
            off += rec_len;
        }
    }
}

fn parse_record(rec: &[u8], f: &mut dyn FnMut(Entry<'_>), errors: &mut Vec<EntryError>) {
    let mut c = Cursor { buf: rec, pos: 4 };
    let Some(ret) = c.take::<attribute_set_t>() else {
        return;
    };
    let mut err = 0u32;
    if ret.commonattr & ATTR_CMN_ERROR != 0 {
        err = c.take::<u32>().unwrap_or(0);
    }
    let mut name: &[u8] = b"";
    if ret.commonattr & ATTR_CMN_NAME != 0 {
        let at = c.pos;
        if let Some(r) = c.take::<attrreference_t>() {
            let start = at as isize + r.attr_dataoffset as isize;
            let len = r.attr_length as usize;
            if start >= 0 && (start as usize) + len <= rec.len() && len > 0 {
                // attr_length includes the trailing NUL.
                name = &rec[start as usize..start as usize + len - 1];
            }
        }
    }
    if err != 0 {
        errors.push((name.to_vec(), err as i32));
        return;
    }
    let dev = if ret.commonattr & ATTR_CMN_DEVID != 0 {
        c.take::<i32>().unwrap_or(0)
    } else {
        0
    };
    let objtype = if ret.commonattr & ATTR_CMN_OBJTYPE != 0 {
        c.take::<u32>().unwrap_or(0)
    } else {
        0
    };
    let mtime = if ret.commonattr & ATTR_CMN_MODTIME != 0 {
        c.take::<libc::timespec>().map(|t| t.tv_sec).unwrap_or(0)
    } else {
        0
    };
    // Bit order: MODTIME (0x400) < FLAGS (0x40000) < FILEID (0x2000000).
    let flags = if ret.commonattr & ATTR_CMN_FLAGS != 0 { c.take::<u32>().unwrap_or(0) } else { 0 };
    let ino = if ret.commonattr & ATTR_CMN_FILEID != 0 {
        c.take::<u64>().unwrap_or(0)
    } else {
        0
    };

    let mut mount_point = false;
    let mut alloc = 0u64;
    let mut nlink = 1u32;
    if ret.dirattr & ATTR_DIR_MOUNTSTATUS != 0 {
        mount_point = c.take::<u32>().unwrap_or(0) & DIR_MNTSTATUS_MNTPOINT != 0;
    }
    if ret.dirattr & ATTR_DIR_ALLOCSIZE != 0 {
        alloc = c.take::<i64>().unwrap_or(0).max(0) as u64;
    }
    if ret.fileattr & ATTR_FILE_LINKCOUNT != 0 {
        nlink = c.take::<u32>().unwrap_or(1);
    }
    if ret.fileattr & ATTR_FILE_ALLOCSIZE != 0 {
        alloc = c.take::<i64>().unwrap_or(0).max(0) as u64;
    }

    let kind = match objtype {
        VREG => Kind::File,
        VDIR => Kind::Dir,
        VLNK => Kind::Symlink,
        _ => Kind::Other,
    };
    f(Entry {
        name,
        kind,
        dev: dev as i64,
        ino,
        nlink,
        alloc,
        mtime,
        mount_point,
        dataless: flags & super::SF_DATALESS != 0,
    });
}
