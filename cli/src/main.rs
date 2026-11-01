//! `spacesense` CLI: scan, report, clean --dry-run.

use spacesense_core::catalog::Safety;
use spacesense_core::plan::{self, Selection, Step};
use spacesense_core::report::{self, Report, ReportOptions};
use spacesense_core::{scan_with_progress, Backend, DirNode, Progress, ScanOptions};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "\
usage:
  spacesense scan   [PATH] [--json] [--top N] [--depth N] [--kib] [--progress]
  spacesense report [PATH] [--json [--plan]] [--no-git] [--all]
  spacesense clean  --dry-run [TARGET...] [--json] [--include-active]

PATH defaults to your home folder.

report   what is using space: caches and tools (catalog rules), project build
         artifacts (node_modules, target/, ...), large files. --all also lists
         rules that found nothing.
clean    prints what would be cleaned, without changing anything (--dry-run is
         required until cleaning lands in M3). With no TARGET it plans the
         default selection: caches, regenerable data and projects untouched for
         30+ days. TARGET is a rule id (see `report`) or a project path.

common options:
  --backend bulk|stat   listing backend (default: bulk = getattrlistbulk)
  --threads N           worker threads (default: one per logical CPU)
  --catalog DIR         load rules from DIR instead of the built-in catalog
";

#[derive(Default)]
struct Args {
    cmd: String,
    root: Option<PathBuf>,
    targets: Vec<String>,
    opts: ScanOptions,
    catalog: Option<PathBuf>,
    json: bool,
    top: usize,
    depth: usize,
    progress: bool,
    kib: bool,
    no_git: bool,
    all: bool,
    with_plan: bool,
    dry_run: bool,
    include_active: bool,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let cmd = match it.next().as_deref() {
        Some(c @ ("scan" | "report" | "clean")) => c.to_string(),
        Some("-h" | "--help") | None => return Err(String::new()),
        Some(other) => return Err(format!("unknown command {other:?}")),
    };
    let mut a = Args {
        cmd,
        top: 10,
        depth: 1,
        ..Default::default()
    };
    let num = |v: Option<String>, flag: &str| -> Result<usize, String> {
        v.ok_or(format!("{flag} needs a value"))?
            .parse()
            .map_err(|_| format!("bad {flag}"))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--backend" => {
                a.opts.backend = it
                    .next()
                    .ok_or("--backend needs a value")?
                    .parse::<Backend>()?
            }
            "--threads" => a.opts.threads = num(it.next(), "--threads")?,
            "--top" => a.top = num(it.next(), "--top")?,
            "--depth" => a.depth = num(it.next(), "--depth")?,
            "--catalog" => a.catalog = Some(it.next().ok_or("--catalog needs a value")?.into()),
            "--json" => a.json = true,
            "--progress" => a.progress = true,
            "--kib" => a.kib = true,
            "--no-git" => a.no_git = true,
            "--all" => a.all = true,
            "--plan" => a.with_plan = true,
            "--dry-run" | "-n" => a.dry_run = true,
            "--include-active" => a.include_active = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s if a.cmd == "clean" => a.targets.push(expand(s)),
            s => a.root = Some(PathBuf::from(s)),
        }
    }
    Ok(a)
}

/// Accept `~/x` and relative paths as project targets; leave rule ids alone.
fn expand(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        return report::home_dir().join(rest).display().to_string();
    }
    if s.contains('/') || s == "." {
        if let Ok(p) = std::fs::canonicalize(s) {
            return p.display().to_string();
        }
    }
    s.to_string()
}

fn human(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn tilde(p: &std::path::Path, home: &std::path::Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn safety_label(s: Safety) -> &'static str {
    match s {
        Safety::Cache => "cache",
        Safety::Regenerable => "regenerable",
        Safety::Review => "review",
        Safety::NeverAuto => "never-auto",
    }
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let root = args.root.clone().unwrap_or_else(report::home_dir);
    match args.cmd.as_str() {
        "scan" => cmd_scan(&args, root),
        "report" => match build_report(&args, root) {
            Ok(r) => {
                if args.json && args.with_plan {
                    // Same shape the app gets from ss_report (ffi/src/lib.rs).
                    let p = plan::build(&r, &Selection::default());
                    println!("{}", serde_json::json!({ "report": r, "plan": p }));
                } else if args.json {
                    println!("{}", serde_json::to_string_pretty(&r).unwrap());
                } else {
                    print_report(&r, args.all);
                }
                ExitCode::SUCCESS
            }
            Err(c) => c,
        },
        "clean" => {
            if !args.dry_run {
                eprintln!(
                    "error: cleaning isn't implemented yet (milestone M3). Use `spacesense clean --dry-run` to see the plan."
                );
                return ExitCode::from(2);
            }
            let r = match build_report(&args, root) {
                Ok(r) => r,
                Err(c) => return c,
            };
            let sel = Selection {
                targets: args.targets.clone(),
                include_active: args.include_active,
            };
            let p = plan::build(&r, &sel);
            if args.json {
                println!("{}", serde_json::to_string_pretty(&p).unwrap());
            } else {
                print_plan(&p, &r);
            }
            ExitCode::SUCCESS
        }
        _ => unreachable!(),
    }
}

fn build_report(args: &Args, root: PathBuf) -> Result<Report, ExitCode> {
    if !args.json {
        eprintln!("scanning {} …", root.display());
    }
    let opts = ReportOptions {
        scan: args.opts.clone(),
        git: !args.no_git,
        catalog_dir: args.catalog.clone(),
        home: None,
        progress: None,
    };
    report::build(&root, &opts).map_err(|e| {
        eprintln!("error: {}: {e}", root.display());
        ExitCode::FAILURE
    })
}

fn print_report(r: &Report, all: bool) {
    let h = &r.home;
    if let Some(v) = &r.volume {
        println!(
            "Disk: {} of {} used, {} free ({}% full)",
            human(v.used_bytes),
            human(v.total_bytes),
            human(v.free_bytes),
            v.used_bytes * 100 / v.total_bytes.max(1)
        );
        if let Some(d) = v.volume_used_bytes {
            println!(
                "      {} on this volume (your files and apps), {} on macOS's other volumes (system, swap, updates)",
                human(d),
                human(v.used_bytes.saturating_sub(d))
            );
        }
    }
    println!(
        "Scanned {}: {} in {} files, {:.1}s{}\n",
        tilde(&r.root, h),
        human(r.scan.total_bytes),
        r.scan.files,
        r.scan.elapsed_ms as f64 / 1000.0,
        if r.needs_full_disk_access.is_empty() {
            String::new()
        } else {
            format!(
                " · {} locations need Full Disk Access",
                r.needs_full_disk_access.len()
            )
        }
    );

    println!("CACHES, TOOLS & APP DATA");
    for x in r
        .rules
        .iter()
        .filter(|x| all || x.size > 0 || x.note.is_some() && x.found)
    {
        let mut flags = Vec::new();
        if x.leftover {
            flags.push("app uninstalled: leftover".to_string());
        }
        if x.tool_missing {
            flags.push(format!(
                "{} not installed",
                x.rule.tool.as_deref().unwrap_or("")
            ));
        }
        for a in &x.running_apps {
            flags.push(format!("{a} running"));
        }
        if let Some(n) = &x.note {
            flags.push(n.clone());
        }
        let flags = if flags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", flags.join("; "))
        };
        let loc = x
            .paths
            .first()
            .map(|p| tilde(&p.path, h))
            .unwrap_or_default();
        println!(
            "{:>10}  {:<28} {:<12} {}{}",
            human(x.size),
            x.name,
            safety_label(x.safety),
            loc,
            flags
        );
        for d in x.details.iter().chain(x.items.iter().take(8)) {
            if d.size >= 50_000_000 {
                println!(
                    "{:>10}    {:<26} {:<12} {}",
                    human(d.size),
                    "",
                    "",
                    tilde(&d.path, h)
                );
            }
        }
    }

    let total: u64 = r.projects.iter().map(|p| p.artifact_bytes).sum();
    let nm: Vec<_> = r
        .projects
        .iter()
        .flat_map(|p| &p.artifacts)
        .filter(|a| a.name == "node_modules")
        .collect();
    println!(
        "\nPROJECTS WITH BUILD ARTIFACTS: {} in {} projects (node_modules: {} in {} folders)",
        human(total),
        r.projects.len(),
        human(nm.iter().map(|a| a.size).sum()),
        nm.len()
    );
    let now = r.generated_at;
    for p in r.projects.iter().take(25) {
        let days = (now - p.last_active) / 86_400;
        let git = match &p.git {
            None => "no git".to_string(),
            Some(g) => {
                let mut s = g.branch.clone().unwrap_or_else(|| "?".into());
                if g.dirty == Some(true) {
                    s += ", dirty";
                }
                if let Some(m) = &g.worktree_of {
                    s += &format!(", worktree of {}", tilde(m, h));
                }
                s
            }
        };
        let arts: Vec<String> = p.artifacts.iter().map(|a| a.name.clone()).collect();
        println!(
            "{:>10}  {:<45} {:>5}d  {}  ({})",
            human(p.artifact_bytes),
            tilde(&p.path, h),
            days,
            arts.join(" "),
            git
        );
    }
    if r.projects.len() > 25 {
        println!(
            "{:>10}  … and {} more (use --json)",
            "",
            r.projects.len() - 25
        );
    }

    println!("\nLARGE FILES (≥ 500 MB, outside caches)");
    for f in r.large_files.iter().take(15) {
        let days = (now - f.mtime) / 86_400;
        println!(
            "{:>10}  {:<11} {:>5}d  {}",
            human(f.size),
            f.kind,
            days,
            tilde(&f.path, h)
        );
    }
}

fn print_plan(p: &plan::Plan, r: &Report) {
    let h = &r.home;
    println!("DRY RUN: nothing will be changed.\n");
    // Small project actions are summarized in one line to keep the plan readable.
    const SMALL: u64 = 20_000_000;
    let (small, shown): (Vec<_>, Vec<_>) = p
        .actions
        .iter()
        .partition(|a| a.source == "project" && a.bytes < SMALL);
    for (i, a) in shown.iter().enumerate() {
        println!(
            "{:>2}. {:>9}  {}  [{}]",
            i + 1,
            human(a.bytes),
            a.title,
            safety_label(a.safety)
        );
        let mut dirs = 0;
        for s in &a.steps {
            match s {
                Step::Run { argv, dry_run_argv } => {
                    println!("              run: {}", argv.join(" "));
                    if let Some(d) = dry_run_argv {
                        println!("              (preview with: {})", d.join(" "));
                    }
                }
                Step::DeleteContents { path } => {
                    println!("              delete contents of {}", tilde(path, h))
                }
                Step::Trash { path } => println!("              move to Trash: {}", tilde(path, h)),
                Step::DeleteDir { path } => {
                    dirs += 1;
                    if dirs <= 4 {
                        println!("              delete {}", tilde(path, h));
                    }
                }
            }
        }
        if dirs > 4 {
            println!("              … and {} more folders", dirs - 4);
        }
        for w in &a.warnings {
            println!("              note: {w}");
        }
        for b in &a.blockers {
            println!("              BLOCKED: {b}");
        }
    }
    if !small.is_empty() {
        println!(
            "{:>2}. {:>9}  {} smaller project folders (under {} each; see --json)  [regenerable]",
            shown.len() + 1,
            human(small.iter().map(|a| a.bytes).sum()),
            small.len(),
            human(SMALL)
        );
    }
    println!(
        "\nEstimated total: {} (actual freed space is re-measured after cleaning)",
        human(p.total_bytes)
    );
    if !p.skipped.is_empty() {
        println!("\nNot included:");
        for s in &p.skipped {
            let t = if s.target.starts_with('/') {
                tilde(std::path::Path::new(&s.target), h)
            } else {
                s.target.clone()
            };
            println!(
                "  {:>9}  {}: {}",
                if s.bytes > 0 {
                    human(s.bytes)
                } else {
                    "-".into()
                },
                t,
                s.reason
            );
        }
    }
}

fn print_tree(n: &DirNode, indent: usize, depth: usize, top: usize) {
    if depth == 0 {
        return;
    }
    for c in n.children.iter().take(top) {
        let err = c
            .error
            .as_deref()
            .map(|e| format!("  [{e}]"))
            .unwrap_or_default();
        println!(
            "{:>10}  {}{}{}",
            human(c.size),
            "  ".repeat(indent),
            c.name,
            err
        );
        print_tree(c, indent + 1, depth - 1, top);
    }
}

fn cmd_scan(args: &Args, root: PathBuf) -> ExitCode {
    let progress = Arc::new(Progress::default());
    let ticker = args.progress.then(|| {
        let p = progress.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            eprint!(
                "\r{} dirs, {} files, {}      ",
                p.dirs.load(Relaxed),
                p.files.load(Relaxed),
                human(p.bytes.load(Relaxed))
            );
        })
    });
    let mut r = match scan_with_progress(&root, &args.opts, &progress) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {}: {e}", root.display());
            return ExitCode::FAILURE;
        }
    };
    if ticker.is_some() {
        eprintln!();
    }
    r.tree.sort_by_size();
    if args.kib {
        println!("{}", r.total_bytes / 1024);
    } else if args.json {
        println!("{}", serde_json::to_string(&r).unwrap());
    } else {
        let s = &r.stats;
        println!(
            "{}  {}  ({} KiB)\n{} files, {} dirs in {:.2}s ({:?}, {} threads)",
            human(r.total_bytes),
            r.root.display(),
            r.total_bytes / 1024,
            s.files,
            s.dirs,
            r.elapsed_ms as f64 / 1000.0,
            r.backend,
            r.threads
        );
        println!(
            "hard links deduped: {} ({}), other-device dirs skipped: {}, unreadable dirs: {}, entry errors: {}\n",
            s.hardlinks_deduped,
            human(s.hardlink_bytes_deduped),
            s.other_device_skipped,
            s.dir_errors,
            s.entry_errors
        );
        print_tree(&r.tree, 0, args.depth, args.top);
    }
    ExitCode::SUCCESS
}
