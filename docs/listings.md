# Listing drafts (submit by hand; they need the owner's accounts)

Links: site https://amarpathak.github.io/spacesense/ · repo https://github.com/amarpathak/spacesense ·
download https://github.com/amarpathak/spacesense/releases/latest · icon `site/icon.png` ·
screenshots `docs/screenshots/*.png` (demo data).

## AlternativeTo (alternativeto.net → Add application)

- **Name:** SpaceSense
- **Alternative to:** CleanMyMac, DaisyDisk, GrandPerspective, OmniDiskSweeper, npkill
- **License / price:** Open source (MIT), free
- **Platforms:** Mac
- **Tags:** disk-space-analyzer, disk-cleaner, developer-tools, node-modules
- **Short description (≤ 140):** Free, open-source Mac app that explains what's filling your disk and cleans it safely, built for developers.
- **Description:**
  SpaceSense shows what every folder on your Mac is, which app owns it (including leftovers
  from uninstalled apps), and whether it's safe to delete: Cache, Rebuilds, Review or Don't
  delete. It knows developer junk (node_modules per project with git state, npm/Yarn/pnpm/
  Bun/pip/Cargo/Go/Homebrew caches, Docker Desktop, Xcode, Playwright, Ollama, Hugging Face)
  and app data like Chrome's on-device AI model. Nothing is ever deleted automatically: you
  tick items, review every exact path and command, and confirm; your files only go to the
  Trash; every step is logged with the space actually freed. Fast Rust scanner (4 M files in
  ~20 s, sizes equal to du), SwiftUI app, CLI included.

## MacUpdate (macupdate.com → Submit app)

Same short description and description; category Utilities › Disk Utilities; requirements
macOS 13+, Apple Silicon and Intel; price Free.

## Reddit r/macapps (post as the developer; follow the sub's self-promotion rules)

**Title:** I made a free, open-source app that explains what's filling your Mac (and cleans only what you pick)

My 500 GB MacBook was 96% full. Finder wouldn't tell me why, and cleaners either scared me
or wanted a subscription. So I built SpaceSense: it explains every folder (what it is, which
app owns it, whether it's safe to delete) and is built for developers. It found a 24 GB Yarn
cache, 15 GB of npm cache, 40 GB of node_modules across 110 projects, a 20 GB Docker disk
image from an app I'd uninstalled, and Chrome's 8.5 GB on-device AI model.

Nothing is deleted automatically: you tick, review the exact steps, confirm. Free, MIT,
Rust + SwiftUI. Not notarized yet (needs "Open Anyway" once). Feedback very welcome,
especially folders it explains wrongly.

https://github.com/amarpathak/spacesense

## Show HN (news.ycombinator.com/submit)

**Title:** Show HN: SpaceSense – explains what's filling your Mac and cleans it with you in the loop
**URL:** https://github.com/amarpathak/spacesense
**First comment:** the story above, plus the technical bits: getattrlistbulk-based parallel
walker in Rust (19.6 s vs diskus 29 s vs du 185 s on 4.2 M files, totals equal to du),
catalog of rules in TOML that also generates the website, executor that re-checks every step
on disk (symlink swaps, git-ignore state, running apps) right before acting.
