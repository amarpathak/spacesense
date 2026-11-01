//! C ABI for the SwiftUI app (PLAN.md §3.2). JSON strings cross the boundary;
//! the header is `app/bridge.h` (kept by hand: three functions).
//!
//! Strings returned by `ss_*` are owned by Rust: free them with `ss_free`.

use spacesense_core::plan::{self, Selection};
use spacesense_core::report::{self, ReportOptions};
use spacesense_core::Progress;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

/// The last report, kept so the app can browse folders without rescanning.
static LAST: Mutex<Option<report::Report>> = Mutex::new(None);
use std::time::Duration;

/// Called from a background thread about 5 times a second while scanning,
/// and once more at the end. `ctx` is passed through untouched.
/// `phase`: 0 reading folders, 1 measuring caches, 2 checking git repositories.
pub type ProgressCb =
    Option<extern "C" fn(ctx: *mut c_void, dirs: u64, files: u64, bytes: u64, phase: u32)>;

struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}

fn to_c(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_else(|_| CString::new("{\"error\":\"NUL in output\"}").unwrap()).into_raw()
}

/// Scan `root` (NULL = home folder) and return
/// `{"report": Report, "plan": Plan}` (the default dry-run plan), or `{"error": "..."}`.
/// Blocks until done; call it off the main thread.
#[no_mangle]
pub extern "C" fn ss_report(root: *const c_char, cb: ProgressCb, ctx: *mut c_void) -> *mut c_char {
    let root = if root.is_null() {
        report::home_dir()
    } else {
        PathBuf::from(unsafe { CStr::from_ptr(root) }.to_string_lossy().into_owned())
    };
    let progress = Arc::new(Progress::default());
    let done = Arc::new(AtomicBool::new(false));
    let ticker = cb.map(|f| {
        let (p, d, ctx) = (progress.clone(), done.clone(), SendPtr(ctx));
        std::thread::spawn(move || {
            let ctx = ctx;
            loop {
                let fin = d.load(Relaxed);
                f(
                    ctx.0,
                    p.dirs.load(Relaxed),
                    p.files.load(Relaxed),
                    p.bytes.load(Relaxed),
                    p.phase.load(Relaxed) as u32,
                );
                if fin {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        })
    });
    let opts = ReportOptions { progress: Some(progress), ..Default::default() };
    let out = std::panic::catch_unwind(|| match report::build(&root, &opts) {
        Ok(r) => {
            let p = plan::build(&r, &Selection::default());
            let json = serde_json::json!({ "report": r, "plan": p }).to_string();
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            json
        }
        Err(e) => serde_json::json!({ "error": format!("{}: {e}", root.display()) }).to_string(),
    })
    .unwrap_or_else(|_| "{\"error\":\"internal error (panic) while scanning\"}".into());
    done.store(true, Relaxed);
    if let Some(t) = ticker {
        let _ = t.join();
    }
    to_c(out)
}

/// Contents of folder `path` from the last scan: subfolders biggest first (at most
/// `limit`), each with what it is and whether it's safe to delete.
/// Returns a FolderListing as JSON, or `{"error": "..."}`.
#[no_mangle]
pub extern "C" fn ss_folder(path: *const c_char, limit: u32) -> *mut c_char {
    if path.is_null() {
        return to_c("{\"error\":\"no path\"}".into());
    }
    let path = PathBuf::from(unsafe { CStr::from_ptr(path) }.to_string_lossy().into_owned());
    let guard = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let out = match guard.as_ref() {
        None => serde_json::json!({ "error": "no scan yet" }).to_string(),
        Some(r) => match r.listing(&path, limit as usize) {
            Some(l) => serde_json::to_string(&l).unwrap_or_default(),
            None => serde_json::json!({ "error": format!("{} is not in the last scan", path.display()) }).to_string(),
        },
    };
    to_c(out)
}

#[no_mangle]
pub extern "C" fn ss_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Static string; do not free.
#[no_mangle]
pub extern "C" fn ss_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}
