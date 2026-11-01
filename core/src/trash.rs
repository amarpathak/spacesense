//! Move to Trash through macOS's own `-[NSFileManager trashItemAtURL:…]`, so the
//! item lands in the user's Trash with "Put Back" support, exactly like Finder.
//! Called through the Objective-C runtime; no extra crates.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

type Id = *mut c_void;
type Sel = *mut c_void;

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

fn sel(name: &str) -> Sel {
    let c = CString::new(name).unwrap();
    unsafe { sel_registerName(c.as_ptr()) }
}

fn class(name: &str) -> Id {
    let c = CString::new(name).unwrap();
    unsafe { objc_getClass(c.as_ptr()) }
}

/// Move `path` to the Trash. Errors carry macOS's own description.
pub fn trash(path: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| "path contains NUL".to_string())?;
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let result = (|| {
            let send_cstr: extern "C" fn(Id, Sel, *const c_char) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let send_id: extern "C" fn(Id, Sel, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let send0: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
            let send_trash: extern "C" fn(Id, Sel, Id, *mut Id, *mut Id) -> u8 =
                std::mem::transmute(objc_msgSend as *const ());
            let send_str: extern "C" fn(Id, Sel) -> *const c_char = std::mem::transmute(objc_msgSend as *const ());

            let ns_path = send_cstr(class("NSString"), sel("stringWithUTF8String:"), c_path.as_ptr());
            if ns_path.is_null() {
                return Err("couldn't convert path".to_string());
            }
            let url = send_id(class("NSURL"), sel("fileURLWithPath:"), ns_path);
            let fm = send0(class("NSFileManager"), sel("defaultManager"));
            let mut err: Id = std::ptr::null_mut();
            let ok = send_trash(fm, sel("trashItemAtURL:resultingItemURL:error:"), url, std::ptr::null_mut(), &mut err);
            if ok != 0 {
                return Ok(());
            }
            if err.is_null() {
                return Err("macOS refused to move it to the Trash".to_string());
            }
            let desc = send0(err, sel("localizedDescription"));
            let s = send_str(desc, sel("UTF8String"));
            Err(if s.is_null() { "move to Trash failed".into() } else { CStr::from_ptr(s).to_string_lossy().into_owned() })
        })();
        objc_autoreleasePoolPop(pool);
        result
    }
}
