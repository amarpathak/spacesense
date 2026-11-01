//! Executes a cleanup plan (PLAN.md §5). Callers must only pass plans built
//! from targets a person selected and confirmed; nothing here decides *what* to
//! clean.
//!
//! Before every step it re-checks, against the disk as it is now:
//! - the path still validates (exists, inside home, not protected, not a symlink);
//! - required apps are quit;
//! - a project build folder is still git-ignored.
//! Each step is measured before and after (actual freed space, not estimates)
//! and appended to the operation log as one JSON line.

use crate::catalog::{running_process_names, run_capture};
use crate::plan::{Action, Plan, Step};
use crate::safety;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct CleanOptions {
    pub home: PathBuf,
    /// Never-auto locations (refused).
    pub protected: Vec<PathBuf>,
    /// JSON-lines operation log (default ~/Library/Logs/SpaceSense/operations.log).
    pub log_path: PathBuf,
    /// Longest an official cleanup command may run.
    pub command_timeout: Duration,
}

impl CleanOptions {
    pub fn new(home: &Path, protected: Vec<PathBuf>) -> CleanOptions {
        CleanOptions {
            home: home.to_path_buf(),
            protected,
            log_path: home.join("Library/Logs/SpaceSense/operations.log"),
            command_timeout: Duration::from_secs(30 * 60),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StepResult {
    pub step: Step,
    pub ok: bool,
    /// Not attempted (a re-check failed).
    pub skipped: bool,
    pub message: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub freed_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ActionResult {
    pub target: String,
    pub title: String,
    pub freed_bytes: u64,
    pub steps: Vec<StepResult>,
}

#[derive(Debug, Serialize)]
pub struct CleanResult {
    pub actions: Vec<ActionResult>,
    pub freed_bytes: u64,
    pub free_before: Option<u64>,
    pub free_after: Option<u64>,
    pub log_path: PathBuf,
}

/// Progress: (index of the step being run, total steps, description).
pub type ProgressFn<'a> = &'a (dyn Fn(usize, usize, &str) + Sync);

pub fn execute(plan: &Plan, opts: &CleanOptions, progress: ProgressFn) -> CleanResult {
    let total: usize = plan.actions.iter().map(|a| a.steps.len()).sum();
    let free_before = free_bytes(&opts.home);
    let mut done = 0;
    let mut results = Vec::new();
    for a in &plan.actions {
        let mut steps = Vec::new();
        let blocker = running_blocker(a);
        for s in &a.steps {
            progress(done, total, &describe(s));
            let r = match &blocker {
                Some(b) => skipped(s, b),
                None => run_step(s, opts),
            };
            log(opts, a, &r);
            steps.push(r);
            done += 1;
        }
        results.push(ActionResult {
            target: a.target.clone(),
            title: a.title.clone(),
            freed_bytes: steps.iter().map(|s| s.freed_bytes).sum(),
            steps,
        });
    }
    progress(total, total, "done");
    CleanResult {
        freed_bytes: results.iter().map(|a| a.freed_bytes).sum(),
        actions: results,
        free_before,
        free_after: free_bytes(&opts.home),
        log_path: opts.log_path.clone(),
    }
}

pub fn describe(s: &Step) -> String {
    match s {
        Step::Run { argv, .. } => {
            let mut v = argv.clone();
            if let Some(first) = v.first_mut() {
                *first = Path::new(first).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            }
            format!("run {}", v.join(" "))
        }
        Step::DeleteContents { path } => format!("empty {}", path.display()),
        Step::DeleteDir { path } => format!("delete {}", path.display()),
        Step::Trash { path } => format!("move to Trash {}", path.display()),
        Step::DeleteOld { path, days } => format!("delete files older than {days} days in {}", path.display()),
    }
}

fn running_blocker(a: &Action) -> Option<String> {
    if a.quit_apps.is_empty() {
        return None;
    }
    let running = running_process_names();
    a.quit_apps.iter().find(|q| running.iter().any(|p| p == *q)).map(|q| format!("{q} is running: quit it first"))
}

fn skipped(s: &Step, why: &str) -> StepResult {
    StepResult {
        step: s.clone(),
        ok: false,
        skipped: true,
        message: why.into(),
        before_bytes: 0,
        after_bytes: 0,
        freed_bytes: 0,
        exit_code: None,
    }
}

fn run_step(s: &Step, opts: &CleanOptions) -> StepResult {
    // Re-validate every path right now; refuse symlinks (deleting through one
    // would hit its target).
    let check = |p: &Path| -> Result<PathBuf, String> {
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("{} is a symlink; refusing", p.display())),
            Ok(_) => {}
            Err(_) => return Err(format!("{} no longer exists", p.display())),
        }
        safety::validate(p, &opts.home, &opts.protected).map_err(|e| format!("{}: {e}", p.display()))
    };
    match s {
        Step::Run { argv, measure, .. } => {
            let before: u64 = measure.iter().map(|p| measure_bytes(p)).sum();
            let Some((bin, args)) = argv.split_first() else { return skipped(s, "empty command") };
            if !Path::new(bin).is_file() {
                return skipped(s, &format!("{bin} not found"));
            }
            let out = run_capture_code(Path::new(bin), args, opts.command_timeout);
            let after: u64 = measure.iter().map(|p| measure_bytes(p)).sum();
            let (ok, code, msg) = match out {
                Some((0, _)) => (true, Some(0), "command finished".to_string()),
                Some((c, tail)) => (false, Some(c), format!("command failed (exit {c}): {tail}")),
                None => (false, None, "command didn't finish in time and was stopped".to_string()),
            };
            result(s, ok, msg, before, after, code)
        }
        Step::DeleteDir { path } => {
            let real = match check(path) {
                Ok(r) => r,
                Err(e) => return skipped(s, &e),
            };
            if let Some(why) = git_tracked(&real) {
                return skipped(s, &why);
            }
            let before = measure_bytes(&real);
            let r = std::fs::remove_dir_all(&real);
            let after = measure_bytes(&real);
            match r {
                Ok(()) => result(s, true, "deleted".into(), before, after, None),
                Err(e) => result(s, false, format!("partly deleted: {e}"), before, after, None),
            }
        }
        Step::DeleteContents { path } => {
            let real = match check(path) {
                Ok(r) => r,
                Err(e) => return skipped(s, &e),
            };
            let before = measure_bytes(&real);
            let mut errors = Vec::new();
            match std::fs::read_dir(&real) {
                Ok(rd) => {
                    for e in rd.flatten() {
                        let p = e.path();
                        // symlink_metadata: never follow links out of the folder.
                        let r = match std::fs::symlink_metadata(&p) {
                            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&p),
                            Ok(_) => std::fs::remove_file(&p),
                            Err(e) => Err(e),
                        };
                        if let Err(err) = r {
                            errors.push(format!("{}: {err}", p.display()));
                        }
                    }
                }
                Err(e) => errors.push(e.to_string()),
            }
            let after = measure_bytes(&real);
            if errors.is_empty() {
                result(s, true, "emptied".into(), before, after, None)
            } else {
                let n = errors.len();
                result(s, false, format!("{n} item(s) couldn't be removed, e.g. {}", errors[0]), before, after, None)
            }
        }
        Step::DeleteOld { path, days } => {
            let real = match check(path) {
                Ok(r) => r,
                Err(e) => return skipped(s, &e),
            };
            let before = measure_bytes(&real);
            let cutoff = std::time::SystemTime::now() - Duration::from_secs(*days as u64 * 86_400);
            let (mut removed, mut failed) = (0u64, 0u64);
            let mut stack = vec![real.clone()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    let Ok(m) = p.symlink_metadata() else { continue };
                    if m.is_dir() {
                        stack.push(p);
                    } else if m.modified().map(|t| t < cutoff).unwrap_or(false) {
                        match std::fs::remove_file(&p) {
                            Ok(()) => removed += 1,
                            Err(_) => failed += 1,
                        }
                    }
                }
            }
            let after = measure_bytes(&real);
            let msg = format!("deleted {removed} file(s) older than {days} days{}", if failed > 0 { format!(", {failed} couldn't be removed") } else { String::new() });
            result(s, failed == 0, msg, before, after, None)
        }
        Step::Trash { path } => {
            let real = match check(path) {
                Ok(r) => r,
                Err(e) => return skipped(s, &e),
            };
            let before = measure_bytes(&real);
            let r = crate::trash::trash(&real);
            let after = measure_bytes(&real);
            match r {
                // Space returns only when the Trash is emptied; still report the move.
                Ok(()) => result(s, true, "moved to the Trash (space is freed when you empty the Trash)".into(), before, after, None),
                Err(e) => result(s, false, e, before, after, None),
            }
        }
    }
}

fn result(s: &Step, ok: bool, message: String, before: u64, after: u64, exit_code: Option<i32>) -> StepResult {
    StepResult {
        step: s.clone(),
        ok,
        skipped: false,
        message,
        before_bytes: before,
        after_bytes: after,
        freed_bytes: before.saturating_sub(after),
        exit_code,
    }
}

/// For a folder inside a git work tree: refuse if git doesn't ignore it
/// (it may hold tracked or uncommitted files). Outside git: fine.
fn git_tracked(dir: &Path) -> Option<String> {
    let parent = dir.parent()?;
    let git = ["/usr/bin/git", "/opt/homebrew/bin/git", "/usr/local/bin/git"].into_iter().map(Path::new).find(|p| p.is_file())?;
    let inside = run_capture(git, &["-C".into(), parent.display().to_string(), "rev-parse".into(), "--is-inside-work-tree".into()], Duration::from_secs(10));
    if inside.as_deref().map(str::trim) != Some("true") {
        return None;
    }
    let ignored = run_capture(git, &["-C".into(), parent.display().to_string(), "check-ignore".into(), "-q".into(), dir.display().to_string()], Duration::from_secs(10));
    match ignored {
        Some(_) => None, // exit 0: ignored
        None => Some(format!("{} is not ignored by git: may contain tracked or uncommitted files", dir.display())),
    }
}

/// Allocated bytes of a path now (0 if gone).
pub fn measure_bytes(p: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(p) {
        Err(_) => 0,
        Ok(m) if !m.is_dir() => m.blocks() * 512,
        Ok(_) => crate::scan(p, &crate::ScanOptions { large_file_threshold: 0, ..Default::default() })
            .map(|r| r.total_bytes)
            .unwrap_or(0),
    }
}

/// Run a command; (exit code, last 300 bytes of stderr+stdout), or None on timeout.
fn run_capture_code(bin: &Path, args: &[String], timeout: Duration) -> Option<(i32, String)> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let t1 = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t2 = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = std::time::Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(-1),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = format!("{}{}", t2.join().unwrap_or_default(), t1.join().unwrap_or_default());
    let tail: String = text.trim().chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
    Some((code, tail))
}

fn free_bytes(p: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail * st.f_bsize as u64)
}

fn log(opts: &CleanOptions, a: &Action, r: &StepResult) {
    #[derive(Serialize)]
    struct Line<'a> {
        time: String,
        target: &'a str,
        title: &'a str,
        step: &'a Step,
        ok: bool,
        skipped: bool,
        message: &'a str,
        before_bytes: u64,
        after_bytes: u64,
        freed_bytes: u64,
        exit_code: Option<i32>,
    }
    let line = Line {
        time: timestamp(),
        target: &a.target,
        title: &a.title,
        step: &r.step,
        ok: r.ok,
        skipped: r.skipped,
        message: &r.message,
        before_bytes: r.before_bytes,
        after_bytes: r.after_bytes,
        freed_bytes: r.freed_bytes,
        exit_code: r.exit_code,
    };
    if let Some(dir) = opts.log_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&opts.log_path) {
        if let Ok(s) = serde_json::to_string(&line) {
            let _ = writeln!(f, "{s}");
        }
    }
}

/// Local time, ISO 8601 with offset.
fn timestamp() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let off = tm.tm_gmtoff / 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        if off < 0 { '-' } else { '+' },
        off.abs() / 60,
        off.abs() % 60
    )
}
