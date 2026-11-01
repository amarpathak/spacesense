# SpaceSense: plan

> A free, open-source Mac app that **explains what is filling your disk and cleans it safely**,
> with first-class support for developer junk (node_modules, package caches, Docker, Xcode,
> AI models). Rust scanning engine, SwiftUI front end.
>
> Written 2026-10-31 to hand off to a fresh session. Everything needed to start is in here.
> Sibling project: FanBoost (`~/fanboost`, github.com/amarpathak/fanboost); reuse its
> packaging and its lessons (section 10).

---

## 1. Why: the owner's own Mac (measured 2026-10-31)

Data volume **95% full: 353 GB used, 20 GB free** (22 GB "available for important usage").
No local Time Machine snapshots. Home: `~/Library` 84 GB, Desktop 28 GB, Downloads 21 GB,
Documents 15 GB, Pictures 15 GB.

| Item | Size | Notes |
|---|---|---|
| `node_modules` | **40.9 GB in 254 folders** | 3× ~2.2 GB copies across `project-a` + its git worktrees (`.worktrees/feature-1`, `.worktrees/feature-2`); `~/.npm/_npx/*/node_modules` 1 GB; `~/Downloads/project-f/node_modules` |
| Yarn v1 cache `~/Library/Caches/Yarn` | **22.7 GB** | `yarn` installed |
| Docker Desktop `~/Library/Containers/com.docker.docker` | **18.9 GB** | `Docker.raw` is **60 GB logical / 19 GB on disk** (sparse). `docker` CLI **not on PATH** |
| npm cache `~/.npm` | **13.6 GB** | |
| Claude desktop `~/Library/Application Support/Claude` | **11.5 GB** | `vm_bundles` = 10.1 GB (see §4) |
| Rest of `~/Library/Caches` | ~11 GB | |
| Chrome profile | 6.2 GB | |
| `.rustup` 1.4, `.cache` 1.4, Cursor 1.2, Homebrew cache 1.1, `.cargo` 0.8, pip 0.5, ms-playwright 0.4, puppeteer 0.3 GB | | |

**Roughly 100 GB is clearly reclaimable** (node_modules + yarn + npm + Docker prune), and
48 GB more (Desktop/Downloads) is worth reviewing. Acceptance test for v1: on this Mac, the
app finds all of the above within a few % of `du`, and safely frees ≥ 60 GB.

Tools present: yarn, npm, bun, brew, ollama (no models), cargo 1.99.1 (**only the
x86_64-apple-darwin target is installed**; run `rustup target add aarch64-apple-darwin`).
Not present: pnpm, docker CLI.

---

## 2. Product principles (what makes it different)

The market (§9) is CleanMyMac (subscription, trust/"scareware" reputation), DaisyDisk (pretty
map, no explanations, no dev awareness), OmniDiskSweeper/GrandPerspective (raw lists), and
CLI dev cleaners (kondo, npkill, Mole). SpaceSense's wedge:

1. **Explain, don't scare.** Every item says what it is, who created it, whether it comes
   back, and what deleting it costs ("re-downloads 10 GB next launch").
2. **Developer-aware.** Knows node_modules, package caches, Docker, Xcode, AI models,
   Playwright/Puppeteer browsers, git worktrees.
3. **Safe by construction.** Dry-run by default, official cleanup commands first, Trash
   for anything user-made, never-auto list, operation log, re-measure after (§5).
4. **Fast and light.** A parallel Rust scanner (seconds, not minutes) and 0% CPU when idle.
5. **Open source (MIT), free.** Trust is the product.

---

## 3. Architecture

```
spacesense/
  core/            Rust crate: scanner, artifact detector, catalog, size accounting
    catalog/*.toml   data-driven rules (one per hog), also the source for SEO pages (§8)
  cli/             `spacesense` CLI (same crate, bin target): scan, report --json, clean --dry-run
  app/             SwiftUI app (macOS 13+), links core as a static library
  packaging/       build universal binaries, .app, .dmg/.zip (or .pkg), signing hooks
  site/            static site generated from catalog (later)
```

### 3.1 Engine (Rust)

From research (sources in §11):
- **Don't use `jwalk`**: archived 2026-08-05, and its README points to `dua-core`. `walkdir` is
  single-threaded. Write a small **parallel walker** (rayon or crossbeam work queue over
  directories).
- **Use `getattrlistbulk(2)` on macOS**: it returns name, type, allocated size, inode,
  link count and device for a whole directory in roughly one syscall per batch, instead of
  readdir + lstat per file. There's a new `getattrlistbulk` crate (0.1.0; claims about 4×
  faster on NVMe) but it's young: benchmark it, or write our own FFI. Keep a readdir + lstat
  fallback.
- Reference speeds: diskus 1.7 s vs `du` 17.8 s cold on 15 GB / 400k files; gdu 4.7 s vs
  `du` 30.6 s cold. **Target:** full scan of `~` on the owner's Mac (about 350 GB, millions
  of files) in **< 30 s cold, < 10 s warm**. Benchmark against `du -sk`, `dust`, `diskus`
  in milestone M0.
  **Revised after M0 (2026-11-01, accepted by owner):** on this Intel i7 with 4.8 M entries
  the scan takes 19.6 s warm, vs diskus 29 s, dust 38 s and `du` 185 s. The limit is the
  kernel (`kern.maxvnodes` 263 k means no scan of `~` is fully warm). New target: **faster
  than diskus on the same machine**; keep < 10 s warm as the goal for Apple Silicon. See
  `bench/M0.md`.

**Size accounting (needs care on APFS):**
- Use **allocated size** (`st_blocks * 512`, what `du` shows), not logical size.
  Docker.raw: 60 GB logical vs 19 GB real on this Mac.
- **Hard links**: count once, deduped by (dev, inode) when `nlink > 1`.
- **Stay on one device**: record `st_dev` at the root and skip other devices. This avoids
  the `/` vs `/System/Volumes/Data` firmlink double counting. Scan from `/Users/<name>`
  (plus selected `/Library` paths later).
- **APFS clones** make summed sizes overstate real usage, and there's no cheap exact fix.
  v1: plain sums plus a footnote. Later: an opt-in clone-aware mode via
  `fcntl(F_LOG2PHYS_EXT)` extent scanning (dua has `--deduplicate-apfs-clones`).
- **Purgeable space**: a volume-level number only
  (`NSURLVolumeAvailableCapacityForImportantUsageKey` minus `volumeAvailableCapacity`).
  Never attribute it to files.

**Artifact (project junk) detector**, modelled on kondo's marker table; don't descend into a
matched artifact:

| Marker | Artifacts |
|---|---|
| `package.json` | `node_modules`, `.next`, `.turbo`, `.parcel-cache`, `.angular`, `.nuxt`, `.svelte-kit`, `dist`? (dist is review-only) |
| `Cargo.toml` | `target` |
| `Podfile` | `Pods` |
| `Package.swift` | `.build`, `.swiftpm` |
| `pom.xml` / `build.gradle` | `target` / `build`, `.gradle` |
| `pubspec.yaml` | `.dart_tool`, `build` |
| `*.py`, `pyproject.toml`, `requirements.txt` | `__pycache__`, `.pytest_cache`, `.mypy_cache`, `.venv`/`venv` (venv is review-only) |
| `CMakeLists.txt` | `build` |
| `*.csproj` | `bin`, `obj` |
| RN (`package.json` + `ios/`) | `ios/Pods`, `ios/build`, `android/build`, `android/.gradle` |

Per project, report: total artifact size, **last active** (newest mtime of source files,
excluding artifacts), **git state** (dirty? is it a worktree? which branch?), and whether
it's a git worktree of another repo. Group worktrees with their main repo (the owner has 3
copies of `project-a` deps).

### 3.2 Rust ↔ Swift bridge

The owner has **Command Line Tools only, no Xcode**: `swiftc` is Swift 5.8. Prefer the
simplest bridge that works with that:
- **Recommended:** a C ABI built with `cbindgen` (static lib, universal via `lipo`) that
  passes **JSON strings** across the boundary, plus a progress callback (a C function
  pointer). For example: `ss_scan_start(root, cb, ctx) -> handle`, `ss_scan_result(handle)
  -> *char JSON`, `ss_free(*char)`. This is easy to debug, needs no generated Swift, and
  works with Swift 5.8 (FanBoost already bridges C via `-import-objc-header`).
- Alternative: UniFFI (typed Swift bindings, async). Check that its generated Swift
  compiles with Swift 5.8 before choosing it.
- The CLI uses the same core, so `spacesense scan --json` gives the same results as the app.

### 3.3 App (SwiftUI, macOS 13+)

A **window app**, not a menu bar app (it's an occasional, task-focused tool). A low-disk
menu bar warning can come later.

Screens:
1. **Overview**: a disk bar (Used / Free / Purgeable) and an explanation of "System Data"
   (§4), then the top categories with size, a one-line explanation and an action button.
   Big honest numbers: "≈ 100 GB safely reclaimable".
2. **Developer**: package caches (npm/yarn/pnpm/bun/pip/brew/cargo/go/gradle), Docker,
   Xcode, AI models, browser test runtimes. Each row shows size, what it is, "comes back
   when…", and the official command it will run.
3. **Projects**: a table of projects with build artifacts (node_modules etc.): size, last
   active, git status, worktree grouping. Projects untouched for more than 30 days are
   preselected; anything active in the last 14 days is never preselected.
4. **Large & old files**: files over 500 MB in Desktop/Downloads/Documents/Movies, with age
   and kind (installers `.dmg/.pkg/.zip`, videos, VM images), plus "Reveal in Finder".
   **Trash only.**
5. **Snapshots & system**: local Time Machine snapshots, Trash size, caches, logs.

All lists stream in while the scan runs. Idle CPU is 0% (no timers when not scanning).

---

## 4. Catalog (v1 rules)

Each rule is a TOML entry: `id, name, paths (with env overrides), detect, size_method,
safety (cache | regenerable | review | never_auto), official_cmd, fallback, comes_back,
explanation, docs_url, verified`. **Verify every path and command on the owner's Mac
before shipping** (the research flagged many defaults as unverified).

| Rule | Path(s) | Official cleanup | Safety | Verified? |
|---|---|---|---|---|
| npm cache | `~/.npm` (`npm config get cache`) | `npm cache clean --force` | cache | docs ✓ |
| Yarn v1 cache | `yarn cache dir` (here: `~/Library/Caches/Yarn`) | `yarn cache clean` | cache | path seen here ✓ |
| Yarn Berry | `~/.yarn/berry` (enableGlobalCache) | `yarn cache clean` | cache | path ✓, command ✗ |
| pnpm store | `pnpm store path` | `pnpm store prune` | cache | docs ✓ (hard-links into projects) |
| bun cache | `bun pm cache` / `~/.bun/install/cache` | `bun pm cache rm` | cache | verify |
| node_modules & JS build caches | detector | delete dir (re-`install`) | regenerable | skip active < 14 days |
| npx cache | `~/.npm/_npx` | part of npm cache | cache | seen here ✓ |
| Docker Desktop | `~/Library/Containers/com.docker.docker/.../Docker.raw` (location is configurable) | `docker system prune` (`-a` opt-in); `docker builder prune`; named volumes **never auto** | review | docs ✓. Measure with allocated size. Docker must be running; CLI may be missing |
| Xcode DerivedData | `~/Library/Developer/Xcode/DerivedData` | delete | regenerable | secondary ✓ |
| Simulators | CoreSimulator | `xcrun simctl delete unavailable` | regenerable | secondary ✓ |
| iOS DeviceSupport | `~/Library/Developer/Xcode/iOS DeviceSupport` | keep newest 2–3 | review | community |
| Xcode Archives | `~/Library/Developer/Xcode/Archives` | — | **never_auto** | |
| Homebrew cache | `brew --cache` | `brew cleanup --prune=all` (`-n` dry run) | cache | docs ✓ |
| pip / uv / conda | `pip cache dir`, `uv cache dir` | `pip cache purge`, `uv cache clean` | cache | verify |
| Cargo | `~/.cargo/registry`, project `target/` | `cargo cache -a` if installed, else delete registry cache | cache | verify |
| Go | `go env GOMODCACHE` | `go clean -modcache` | cache | verify |
| Gradle / Maven / CocoaPods | `~/.gradle/caches`, `~/.m2/repository`, `~/Library/Caches/CocoaPods` | `pod cache clean --all` | cache | verify |
| Ollama models | `~/.ollama/models` (`OLLAMA_MODELS`) | `ollama rm <model>` per model | review (big re-download) | docs ✓ path |
| Hugging Face | `~/.cache/huggingface/hub` (`HF_HOME`, `HF_HUB_CACHE`) | `hf cache ls/rm/prune --dry-run` | review | docs ✓ |
| LM Studio models | verify | — | review | ✗ |
| Playwright browsers | `~/Library/Caches/ms-playwright` | `npx playwright uninstall --all` (doesn't remove folders, so also delete dir) | regenerable | secondary ✓ |
| Puppeteer | `~/.cache/puppeteer` (`PUPPETEER_CACHE_DIR`) | delete | regenerable | ✓ |
| **Claude desktop VM** | `~/Library/Application Support/Claude/vm_bundles/claudevm.bundle` (`rootfs.img` ~10 GB, `sessiondata.img` ~1.3 GB) | none official | **review**: Linux VM for local agent mode/Cowork; re-downloads ~10 GB; deleting `sessiondata.img` loses Cowork history. Quit Claude first | GitHub issues only |
| App caches | `~/Library/Caches/*` (per app), Chrome/Slack/Discord/VS Code/Cursor/Spotify caches | delete per app; app must be quit | cache | verify each |
| Logs | `~/Library/Logs` | delete old (> 30 days) | cache | |
| Trash | `~/.Trash` | empty (confirm) | review | |
| Local TM snapshots | `tmutil listlocalsnapshots /` | `tmutil deletelocalsnapshots <date>`, **needs admin** → run via `osascript … with administrator privileges`; re-measure, may need reboot | review | syntax ✓ |
| iOS backups | `~/Library/Application Support/MobileSync/Backup` (needs Full Disk Access) | — | **never_auto** (may be the only copy) | ✗ |
| Mail/Messages attachments, Photos library | TCC-protected | — | **never_auto**, show size only if FDA granted | |
| Spotlight index | `~/Library/Metadata/CoreSpotlight` | — | **never touch** | |

**"System Data" explainer** (for the Overview and an SEO page): Apple's Storage pane lumps
caches, logs, VM/swap files, app support data, temporary files, local snapshots and iOS
backups into "System Data". Apple says you can't manage it directly. SpaceSense breaks it
into named, explained pieces. Honesty note: Apple's own docs didn't surface in research, so
cite secondary sources and test claims.

**Full Disk Access**: needed for Mail, Messages, Safari, `~/Library/Cookies`,
CoreSpotlight, and probably MobileSync. v1 works without FDA. Show "N locations need Full
Disk Access to measure" with a button that opens the settings pane
(`x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`). Detect
access by trying to read a protected path.

---

## 5. Safety model (non-negotiable)

1. **Scan is read-only.** Nothing changes without an explicit click plus a confirmation
   sheet listing exact paths, sizes and the command that will run.
2. **Dry run first**: the confirmation shows what would happen; official commands get
   their own dry-run flags where they exist (`brew cleanup -n`, `hf cache prune --dry-run`).
3. **Prefer official commands** over `rm`. Fall back to deleting the directory only for
   pure caches, and only if the tool isn't installed.
4. **Caches/regenerable → delete**; **user files → Trash** (`FileManager.trashItem`), so
   they're recoverable.
5. **Never-auto list**: Archives, iOS backups, named Docker volumes, Photos/Mail/Messages,
   Documents files, CoreSpotlight, anything outside the catalog or detector.
6. **Path validation**: resolve symlinks; refuse anything outside `$HOME` or the rule's
   declared roots; refuse `/`, `$HOME` itself, and any path with fewer than 3 components.
7. **Project guards**: skip projects active within 14 days (default), warn on git-dirty
   repos, and treat worktrees as part of their repo.
8. **Apps must be quit** before their caches are cleaned (check `NSRunningApplication`).
9. **Operation log** at `~/Library/Logs/SpaceSense/operations.log`: timestamp, rule,
   paths, bytes freed, command, exit code.
10. **Re-measure after** each cleanup and show *actual* freed space, not estimated.

---

## 6. Milestones

| # | Deliverable | Done when |
|---|---|---|
| **M0** | Engine spike: parallel walker, `getattrlistbulk` vs readdir+lstat, allocated-size + hard-link + one-device accounting. Bench vs `du`, `dust`, `diskus` on `~` | Totals within 1–2% of `du -sk` on the owner's `~`; meets speed target (§3.1) |
| **M1** | `spacesense` CLI: `scan`, `report --json`, artifact detector, catalog loaded from TOML; `clean --dry-run` prints planned actions | Finds the §1 items on the owner's Mac with matching sizes |
| **M2** | SwiftUI app, read-only: Overview, Developer, Projects, Large files, Snapshots; streaming progress | Owner can see where the 353 GB went, with explanations |
| **M3** | Cleanup actions + safety model (§5) + operation log + re-measure | Owner frees ≥ 60 GB safely; nothing outside the plan is touched (verify with log + `du` before/after) |
| **M4** | Packaging: universal app (x86_64 + arm64 Rust slices + Swift), ad-hoc signed, `.dmg`/`.zip` (no root needed, so no `.pkg`); GitHub release | Fresh download → open → works, after the Gatekeeper "Open Anyway" step |
| **M5** | SEO/GEO site generated from the catalog (§8); Homebrew cask; listings | Pages live, README links them |

Work in this order. Each milestone ends with a measured check on the owner's Mac.

---

## 7. Testing

- **Golden numbers**: compare every rule's size against `du -sk <path>` (allocated) on the
  owner's Mac.
- **Fixture tree** (in `core/tests`): hard links, a sparse file (`mkfile -n`), an APFS clone
  (`cp -c`), symlink loops, permission-denied dirs, a fake project per marker type, a git
  worktree, and an active vs stale project (set mtimes with `touch -t`).
- **Cleanup tests run only on fixtures in a temp dir**, never on real caches. Test
  path-validation refusals explicitly (`/`, `$HOME`, symlink pointing outside).
- **Performance**: scan time cold (after `sudo purge`, ask the owner) and warm; app idle
  CPU via `ps -o utime=,stime=` over 60 s (FanBoost lesson).
- **UI**: render SwiftUI views to PNG with `ImageRenderer` in a scratch harness to check
  layout (AppKit controls render as placeholders there, which is expected).

---

## 8. SEO / GEO

- **One page per symptom, generated from the catalog**, answer-first: "What is X / is it
  safe to delete / how much space / the exact command / what SpaceSense does". Real
  measured numbers ("Yarn cache: 22.7 GB on a 2019 MacBook Pro"). AI assistants cite
  concrete, checkable pages.
- Target phrasings seen in research: "System Data taking up space Mac", "how to delete
  system data mac", "what is system data on mac", "mac storage full", plus per-tool ones
  ("clear yarn cache mac", "npm cache size", "node_modules taking up space", "Docker.raw
  huge", "DerivedData delete", "Claude vm_bundles"). Autocomplete data wasn't accessible,
  so validate demand with Google Search Console after launch.
- The GitHub README doubles as a landing page; the repo name and description carry the
  problem words.
- Distribution: Homebrew cask, AlternativeTo (as an alternative to CleanMyMac/DaisyDisk),
  MacUpdate, and helpful answers on Reddit/Stack Exchange/Apple Community.
- Shared brand with FanBoost: cross-link both READMEs.

---

## 9. Competitors (from research)

| App | Model | Strength | Weakness |
|---|---|---|---|
| CleanMyMac (MacPaw) | Subscription (~$3.33/mo+) | Polish, breadth | Price; trust/"scareware" reputation |
| DaisyDisk | ~$9.99 once | Beautiful sunburst map | No explanations, no dev awareness |
| OmniDiskSweeper | Free | Simple list | Deletes permanently, dated |
| GrandPerspective | Free | Treemap | Raw, no guidance |
| DevCleaner for Xcode | Free, OSS | Xcode only | Depends on undocumented folder layouts |
| Mole (tw93) | CLI GPL-3 free + paid app | Good safety model (dry-run, whitelists, op log, skips active projects) | CLI-first |
| kondo / npkill | Free CLI | Project artifacts | No caches, no GUI, limited safety checks |

---

## 10. Lessons carried over from FanBoost (avoid repeating these)

**Toolchain**: Command Line Tools only (no Xcode), Swift 5.8. No `@Observable` (use
`ObservableObject`), no `if`/`switch` expressions. With multiple Swift files, use an
`@main` type (top-level code only works in `main.swift`). SwiftUI + Swift Charts need
`-target …-macos13`. Build universal: compile per arch, then `lipo`. Ad-hoc sign the app
(`codesign --force -s -`); Apple Silicon refuses unsigned code.

**Packaging** (`~/fanboost/package.sh` and `packaging/` are the template):
- Pick the final bundle id from day one (e.g. `io.github.amarpathak.spacesense`).
  Changing it later made macOS Installer put the new app in `/Applications/X.localized/`
  instead of replacing the old one.
- Set `BundleIsRelocatable=false` in the component plist, or Installer may "upgrade" a
  copy of the app found elsewhere on disk (like the build folder).
- Installer HTML needs `<meta charset="utf-8">`, or emoji show as garbage.
- `com.apple.provenance` xattrs show up as `._` entries in `pkgutil --payload-files`; they
  turn back into xattrs on install, which is harmless.
- SpaceSense needs no root (except snapshot deletion via an `osascript` admin prompt), so
  ship a `.zip`/`.dmg` app, not a `.pkg`, and skip sudoers.
- Unsigned downloads hit Gatekeeper's "Apple could not verify…"; document **System Settings
  → Privacy & Security → Open Anyway**. A $99 Developer ID removes this; worth it with 2–3
  apps.

**Performance**: FanBoost 1.1 first used 7–9% CPU because it started a process every
second, ran `ps` every 3 s and redrew charts every second. The fix brought it to about
0.3%: in-process reads, sampling only when needed, UI updates every 3 s, smoothing. Measure
with `ps -o utime=,stime=` over 60 s and profile with `sample <pid> 10`.

**Process / testing gotchas**:
- `UserDefaults` written by a running app may not be on disk yet; read with
  `defaults export <domain> -`, not by parsing the plist file.
- Test harnesses with their own defaults domain can change real system state (one reset
  the battery charge limit). Seed their settings, or make tests side-effect free.
- When load-testing, use self-terminating workers (`perl -e '$e=time+N; 1 while time<$e' &`)
  plus a `trap` cleanup. A backgrounded subshell's `kill $(jobs -p)` didn't work and left
  the CPU at 100 °C.
- The owner runs installs that need a password with `! sudo …` in the prompt; Claude can't
  enter passwords. Batch changes so the owner installs once.
- Handle SIGTERM (`DispatchSource.makeSignalSource`) so the app always shuts down cleanly.

---

## 11. Sources

Engine: github.com/KSXGitHub/parallel-disk-usage · github.com/Byron/dua-cli ·
github.com/bootandy/dust · github.com/sharkdp/diskus · github.com/dundee/gdu ·
dev.yorhel.nl/ncdu · github.com/Byron/jwalk (archived) · docs.rs/getattrlistbulk ·
eclecticlight.co/2022/12/30/free-space-on-an-apfs-volume-is-an-illusion/ ·
eclecticlight.co/2023/07/22/how-macos-depends-on-firmlinks/ · github.com/tbillington/kondo
(kondo-lib/src/lib.rs) · github.com/voidcosmos/npkill ·
stadiamaps.com/blog/ferrostar-building-a-cross-platform-navigation-sdk-in-rust-part-2/

Catalog: docs.npmjs.com · pnpm.io · yarnpkg.com · docs.docker.com/engine/manage-resources/pruning/ ·
docs.docker.com/desktop/settings-and-maintenance/settings/ · docs.brew.sh/Manpage ·
docs.ollama.com/faq · huggingface.co/docs/huggingface_hub/guides/manage-cache ·
browserstack.com/guide/uninstall-playwright · eclecticlight.co/2020/01/22/time-machine-11-tmutil/ ·
developer.apple.com/forums/thread/81171 · discussions.apple.com/thread/255065971 ·
github.com/anthropics/claude-code/issues/65577 · github.com/anthropics/claude-code/issues/47039 ·
fileside.app/blog/2024-05-31_full-disk-access · github.com/tw93/Mole ·
github.com/xenodium/xcode-dev-cleaner · macpaw.com/reviews/daisydisk-alternatives

---

## 12. Open questions for the owner

1. Name: is "SpaceSense" free (GitHub, App Store, trademark)? Check before creating the repo.
2. Scope of v1 scan roots: just `~`, or also `/Library/Caches` and `/private/var/folders`
   (system caches; some need admin)?
3. Should cleaning the owner's own Mac happen *before* the app exists (by hand, in this
   session's style), or be the app's first real test (M3)? Disk is at 95%, and macOS
   gets unstable under about 10 GB free.
