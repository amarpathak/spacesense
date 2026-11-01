//! SpaceSense scanning engine.
//!
//! Size accounting rules (PLAN.md §3.1):
//! - allocated size (`st_blocks * 512`, what `du` shows), never logical length;
//! - hard-linked files (nlink > 1) are counted once, deduped by (dev, inode);
//! - the scan stays on the root's device: mount points and other devices are skipped;
//! - symlinks are never followed (their own allocation is counted, like `du -P`).

pub mod backend;
pub mod catalog;
pub mod explain;
pub mod markers;
pub mod plan;
pub mod projects;
pub mod report;
pub mod safety;

pub use backend::{Backend, Kind};

use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::{CString, OsStr};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub backend: Backend,
    /// Worker threads; 0 = rayon default (one per logical CPU).
    pub threads: usize,
    /// Files at least this big (allocated) are listed in `DirNode::large_files`; 0 = off.
    pub large_file_threshold: u64,
    /// Folders never entered, as absolute paths (default: cloud storage folders
    /// of $HOME). They appear in the tree with `SKIPPED` as their error.
    pub skip: Vec<PathBuf>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            backend: Backend::Bulk,
            threads: 0,
            large_file_threshold: 500_000_000,
            skip: default_skip(),
        }
    }
}

/// Error text on folders that are deliberately not scanned.
pub const SKIPPED: &str = "not scanned: cloud storage (iCloud Drive, Dropbox, Google Drive…)";

/// iCloud Drive and File Provider cloud folders: their content mostly lives in
/// the cloud, and reading it can trigger downloads. Not what slows a Mac down.
pub fn default_skip() -> Vec<PathBuf> {
    match std::env::var_os("HOME") {
        Some(h) => {
            let h = PathBuf::from(h);
            vec![h.join("Library/Mobile Documents"), h.join("Library/CloudStorage")]
        }
        None => Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LargeFile {
    pub name: String,
    pub size: u64,
    pub mtime: i64,
}

/// A directory in the result tree. Only directories are kept; files are summed.
#[derive(Debug, Serialize)]
pub struct DirNode {
    pub name: String,
    /// Allocated bytes of this directory and everything below it.
    pub size: u64,
    /// Non-directory entries below this directory (inclusive).
    pub files: u64,
    /// Newest mtime of anything directly in or below this directory.
    pub newest_mtime: i64,
    /// Newest mtime of the non-directory entries directly in this directory,
    /// ignoring OS metadata such as .DS_Store (0 if none).
    pub files_mtime: i64,
    /// Bitset of `markers::*` found directly in this directory.
    #[serde(skip_serializing_if = "is_zero")]
    pub markers: u32,
    /// Files directly in this directory at or above `ScanOptions::large_file_threshold`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub large_files: Vec<LargeFile>,
    /// The directory couldn't be listed (permission denied, vanished...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<DirNode>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Default, Serialize)]
pub struct ScanStats {
    pub files: u64,
    pub dirs: u64,
    /// Directories that could not be opened or read.
    pub dir_errors: u64,
    /// Entries inside readable directories that could not be stat'ed.
    pub entry_errors: u64,
    /// Extra links to an already-counted inode (not added to the total).
    pub hardlinks_deduped: u64,
    pub hardlink_bytes_deduped: u64,
    /// Directories skipped because they are on another device / are mount points.
    pub other_device_skipped: u64,
    /// Cloud-only folders (iCloud Drive placeholders) not descended into.
    pub cloud_only_dirs: u64,
}

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub root: PathBuf,
    pub backend: Backend,
    pub threads: usize,
    pub elapsed_ms: u128,
    pub total_bytes: u64,
    pub stats: ScanStats,
    pub tree: DirNode,
}

/// Live counters, readable while a scan runs (for progress display).
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    /// `PHASE_*`: what a report is doing (the scan itself only uses 0).
    pub phase: AtomicU64,
}

pub const PHASE_SCANNING: u64 = 0;
/// Measuring catalog locations outside the tree, resolving tool paths.
pub const PHASE_MEASURING: u64 = 1;
/// Running git status / check-ignore for projects.
pub const PHASE_GIT: u64 = 2;

const SHARDS: usize = 64;

struct Ctx<'a> {
    backend: Backend,
    /// `ScanOptions::skip`, relative to the scan root.
    skip: Vec<PathBuf>,
    large: u64,
    root_dev: i64,
    inodes: Vec<Mutex<HashSet<(i64, u64)>>>,
    progress: &'a Progress,
    dir_errors: AtomicU64,
    entry_errors: AtomicU64,
    hl_dups: AtomicU64,
    hl_bytes: AtomicU64,
    other_dev: AtomicU64,
    cloud_only: AtomicU64,
}

/// Error text on folders that are cloud-only placeholders.
pub const CLOUD_ONLY: &str = "in iCloud only (not downloaded to this Mac)";

/// Never let a scan download cloud-only files: with this policy, the kernel
/// refuses to materialize iCloud/File Provider placeholders for this process
/// instead of fetching them (setiopolicy_np(3)). Process-wide and permanent,
/// which is what a disk scanner wants.
fn disable_cloud_materialization() {
    extern "C" {
        fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
    }
    const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
    const IOPOL_SCOPE_PROCESS: libc::c_int = 0;
    const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;
    unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_PROCESS,
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
        );
    }
}

impl Ctx<'_> {
    /// True the first time this (dev, ino) is seen.
    fn first_link(&self, dev: i64, ino: u64) -> bool {
        let shard = (ino as usize ^ (ino >> 32) as usize) % SHARDS;
        self.inodes[shard].lock().unwrap().insert((dev, ino))
    }
}

pub fn scan(root: &Path, opts: &ScanOptions) -> io::Result<ScanResult> {
    scan_with_progress(root, opts, &Progress::default())
}

pub fn scan_with_progress(
    root: &Path,
    opts: &ScanOptions,
    progress: &Progress,
) -> io::Result<ScanResult> {
    let start = Instant::now();
    let meta = std::fs::symlink_metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scan root is not a directory",
        ));
    }
    use std::os::unix::fs::MetadataExt;
    let ctx = Ctx {
        backend: opts.backend,
        skip: opts.skip.iter().filter_map(|p| p.strip_prefix(root).ok().map(Path::to_path_buf)).collect(),
        large: if opts.large_file_threshold == 0 {
            u64::MAX
        } else {
            opts.large_file_threshold
        },
        root_dev: meta.dev() as i64,
        inodes: (0..SHARDS).map(|_| Mutex::new(HashSet::new())).collect(),
        progress,
        dir_errors: AtomicU64::new(0),
        entry_errors: AtomicU64::new(0),
        hl_dups: AtomicU64::new(0),
        hl_bytes: AtomicU64::new(0),
        other_dev: AtomicU64::new(0),
        cloud_only: AtomicU64::new(0),
    };
    disable_cloud_materialization();
    // Same rule as for subdirectories: `du` counts the directory's own blocks.
    let root_own = meta.blocks() * 512;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads)
        // Deep trees recurse through rayon; give workers room.
        .stack_size(16 * 1024 * 1024)
        .build()
        .map_err(io::Error::other)?;
    let threads = pool.current_num_threads();
    raise_fd_limit();
    let root_bytes = root.as_os_str().as_bytes();
    let rel = (!ctx.skip.is_empty()).then(PathBuf::new);
    let mut tree = pool.install(|| walk(&ctx, None, root_bytes, rel, root_own, meta.mtime()));
    tree.name = root.display().to_string();

    let stats = ScanStats {
        files: progress.files.load(Relaxed),
        dirs: progress.dirs.load(Relaxed),
        dir_errors: ctx.dir_errors.load(Relaxed),
        entry_errors: ctx.entry_errors.load(Relaxed),
        hardlinks_deduped: ctx.hl_dups.load(Relaxed),
        hardlink_bytes_deduped: ctx.hl_bytes.load(Relaxed),
        other_device_skipped: ctx.other_dev.load(Relaxed),
        cloud_only_dirs: ctx.cloud_only.load(Relaxed),
    };
    Ok(ScanResult {
        root: root.to_path_buf(),
        backend: opts.backend,
        threads,
        elapsed_ms: start.elapsed().as_millis(),
        total_bytes: tree.size,
        stats,
        tree,
    })
}

/// Walk the directory `name` inside the open directory `parent` (`None`: `name`
/// is the root path). The directory's fd stays open while its children are
/// walked, so they are opened with `openat` relative to it.
fn walk(
    ctx: &Ctx,
    parent: Option<BorrowedFd<'_>>,
    name: &[u8],
    // Path relative to the root, tracked only while it can still lead to a
    // skipped folder (so almost never).
    rel: Option<PathBuf>,
    own_alloc: u64,
    own_mtime: i64,
) -> DirNode {
    ctx.progress.dirs.fetch_add(1, Relaxed);
    ctx.progress.bytes.fetch_add(own_alloc, Relaxed);
    let mut node = DirNode {
        name: String::from_utf8_lossy(name).into_owned(),
        size: own_alloc,
        files: 0,
        newest_mtime: own_mtime,
        files_mtime: 0,
        markers: 0,
        large_files: Vec::new(),
        error: None,
        children: Vec::new(),
    };
    let fd = match CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name contains NUL"))
        .and_then(|c| backend::open_dir_at(parent, &c))
    {
        Ok(fd) => fd,
        Err(e) => {
            node.error = Some(e.to_string());
            ctx.dir_errors.fetch_add(1, Relaxed);
            return node;
        }
    };

    let mut subdirs: Vec<(Vec<u8>, u64, i64)> = Vec::new();
    let mut cloud_dirs: Vec<DirNode> = Vec::new();
    let mut file_bytes = 0u64;
    let mut files = 0u64;
    let mut files_mtime = 0i64;
    let mut marks = 0u32;
    let mut large = Vec::new();
    let res = ctx.backend.read_dir(fd.as_raw_fd(), &mut |e| {
        marks |= markers::marker_for(e.name, e.kind);
        match e.kind {
            Kind::Dir => {
                if e.mount_point || e.dev != ctx.root_dev {
                    ctx.other_dev.fetch_add(1, Relaxed);
                } else if e.dataless {
                    ctx.cloud_only.fetch_add(1, Relaxed);
                    cloud_dirs.push(DirNode {
                        name: String::from_utf8_lossy(e.name).into_owned(),
                        size: e.alloc,
                        files: 0,
                        newest_mtime: e.mtime,
                        files_mtime: 0,
                        markers: 0,
                        large_files: Vec::new(),
                        error: Some(CLOUD_ONLY.into()),
                        children: Vec::new(),
                    });
                } else {
                    subdirs.push((e.name.to_vec(), e.alloc, e.mtime));
                }
            }
            _ => {
                files += 1;
                if !is_os_metadata(e.name) {
                    files_mtime = files_mtime.max(e.mtime);
                }
                if e.nlink > 1 && !ctx.first_link(e.dev, e.ino) {
                    ctx.hl_dups.fetch_add(1, Relaxed);
                    ctx.hl_bytes.fetch_add(e.alloc, Relaxed);
                } else {
                    file_bytes += e.alloc;
                    if e.alloc >= ctx.large {
                        large.push(LargeFile {
                            name: String::from_utf8_lossy(e.name).into_owned(),
                            size: e.alloc,
                            mtime: e.mtime,
                        });
                    }
                }
            }
        }
    });
    node.markers = marks;
    node.files_mtime = files_mtime;
    node.large_files = large;
    match res {
        Ok(errs) if !errs.is_empty() => {
            ctx.entry_errors.fetch_add(errs.len() as u64, Relaxed);
        }
        Ok(_) => {}
        Err(e) => {
            node.error = Some(e.to_string());
            ctx.dir_errors.fetch_add(1, Relaxed);
        }
    }
    ctx.progress.files.fetch_add(files, Relaxed);
    ctx.progress.bytes.fetch_add(file_bytes, Relaxed);

    let dir_fd = fd.as_fd();
    let mut children: Vec<DirNode> = subdirs
        .into_par_iter()
        .map(|(n, alloc, mtime)| {
            let child_rel = rel.as_ref().map(|r| r.join(OsStr::from_bytes(&n)));
            if let Some(cr) = &child_rel {
                if ctx.skip.iter().any(|s| s == cr) {
                    return DirNode {
                        name: String::from_utf8_lossy(&n).into_owned(),
                        size: alloc,
                        files: 0,
                        newest_mtime: mtime,
                        files_mtime: 0,
                        markers: 0,
                        large_files: Vec::new(),
                        error: Some(SKIPPED.into()),
                        children: Vec::new(),
                    };
                }
            }
            let child_rel = child_rel.filter(|cr| ctx.skip.iter().any(|s| s.starts_with(cr)));
            walk(ctx, Some(dir_fd), &n, child_rel, alloc, mtime)
        })
        .collect();
    drop(fd);
    children.append(&mut cloud_dirs);

    node.size += file_bytes + children.iter().map(|c| c.size).sum::<u64>();
    node.files = files + children.iter().map(|c| c.files).sum::<u64>();
    node.newest_mtime = children
        .iter()
        .map(|c| c.newest_mtime)
        .fold(own_mtime.max(files_mtime), i64::max);
    node.children = children;
    node
}

/// Files that macOS/Finder write on their own (browsing a folder updates
/// .DS_Store); they say nothing about when *you* last worked somewhere.
fn is_os_metadata(name: &[u8]) -> bool {
    matches!(
        name,
        b".DS_Store" | b".localized" | b"Icon\r" | b".com.apple.timemachine.donotpresent"
    ) || name.starts_with(b"._")
}

/// Each in-progress directory holds an fd; make sure the soft limit is high.
fn raise_fd_limit() {
    unsafe {
        let mut rl: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) == 0 {
            // macOS caps at OPEN_MAX for setrlimit even when the hard limit is "unlimited".
            let want = rl.rlim_max.min(10240 /* OPEN_MAX */).max(rl.rlim_cur);
            if want > rl.rlim_cur {
                rl.rlim_cur = want;
                libc::setrlimit(libc::RLIMIT_NOFILE, &rl);
            }
        }
    }
}

impl DirNode {
    /// The descendant at relative path `rel` (empty = self).
    pub fn find(&self, rel: &Path) -> Option<&DirNode> {
        let mut n = self;
        for comp in rel.components() {
            let name = comp.as_os_str().to_str()?;
            n = n.children.iter().find(|c| c.name == name)?;
        }
        Some(n)
    }

    /// Sort children by size, largest first, recursively.
    pub fn sort_by_size(&mut self) {
        self.children
            .sort_unstable_by_key(|x| std::cmp::Reverse(x.size));
        for c in &mut self.children {
            c.sort_by_size();
        }
    }
}
