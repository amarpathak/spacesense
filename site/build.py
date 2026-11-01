#!/usr/bin/env python3
"""Builds the SpaceSense website from the catalog (PLAN.md §8): a landing page,
one answer-first page per catalog rule ("What is X on Mac? Is it safe to
delete?"), a "System Data" explainer and a node_modules guide. Static HTML,
no dependencies (Python 3.11+ for tomllib).

usage: python3 site/build.py [OUT_DIR]   (default: site/out)
"""
import html
import json
import shutil
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "site" / "out"
BASE = __import__("os").environ.get("SITE_BASE", "https://amarpathak.github.io/spacesense/")
REPO = "https://github.com/amarpathak/spacesense"
RELEASES = REPO + "/releases/latest"
VERSION = (ROOT / "VERSION").read_text().strip()

# Sizes measured on the author's 2019 MacBook Pro (500 GB), 2026-11-01.
MEASURED = {
    "yarn-v1-cache": "24.4 GB",
    "docker-desktop": "20.3 GB (Docker itself was already uninstalled)",
    "npm-cache": "14.6 GB (2.0 GB of it npx)",
    "claude-vm": "10.8 GB",
    "chrome-ai-model": "8.5 GB (two 4.27 GB versions)",
    "app-caches": "36 GB in total, 9.7 GB outside the caches above",
    "nvm-versions": "2.9 GB",
    "rustup-toolchains": "1.5 GB",
    "homebrew-cache": "1.2 GB",
    "yarn-berry-cache": "1.1 GB",
    "cargo-registry": "0.9 GB",
    "uv-cache": "0.65 GB",
    "pip-cache": "0.58 GB",
    "playwright-browsers": "0.48 GB",
    "logs": "0.47 GB",
    "bun-cache": "0.43 GB",
    "puppeteer-browsers": "0.27 GB",
}

VERDICT = {
    "cache": ("Yes, it's safe to delete.", "A cache: deleting it only costs download or rebuild time.", "ok"),
    "regenerable": ("Yes, it's safe to delete.", "Your tools regenerate it automatically.", "ok"),
    "review": ("Yes, but look first.", "It's safe for the system, but it may be a big re-download or hold something you want.", "warn"),
    "never_auto": ("Don't delete it by hand.", "SpaceSense only shows its size; manage it where the explanation says.", "no"),
}

CATEGORY_TITLE = {
    "package-cache": "Package caches",
    "docker": "Docker",
    "xcode": "Xcode",
    "ai-models": "AI models",
    "browser-runtime": "Browser test runtimes",
    "toolchain": "Toolchains",
    "app-cache": "App caches",
    "app-data": "App data",
    "system": "System",
}

e = html.escape


def page(title, description, body, path, jsonld=None):
    canonical = BASE + path
    ld = f'<script type="application/ld+json">{json.dumps(jsonld)}</script>' if jsonld else ""
    return f"""<!doctype html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{e(title)}</title>
<meta name="description" content="{e(description)}">
<link rel="canonical" href="{canonical}">
<meta property="og:title" content="{e(title)}"><meta property="og:description" content="{e(description)}">
<meta property="og:type" content="website"><meta property="og:url" content="{canonical}">
<meta property="og:image" content="{BASE}screenshots/overview.png">
<link rel="icon" href="{BASE}icon.png">
<link rel="stylesheet" href="{BASE}style.css">{ld}
</head><body>
<header><a class="brand" href="{BASE}"><img src="{BASE}icon.png" alt="" width="28" height="28">SpaceSense</a>
<nav><a href="{BASE}guides/">Guides</a><a href="{BASE}system-data.html">System Data</a><a href="{REPO}">GitHub</a><a class="btn" href="{RELEASES}">Download</a></nav></header>
<main>{body}</main>
<footer><p>SpaceSense is free and open source (MIT). Every page here is generated from the
<a href="{REPO}/tree/main/core/catalog">open catalog</a> the app uses. Found a mistake?
<a href="{REPO}/issues">Open an issue</a>.</p></footer>
</body></html>
"""


def cta():
    return f"""<div class="cta"><div><b>SpaceSense finds and explains all of this for you.</b>
Free Mac app: shows what each folder is and whether it's safe to delete, and cleans only what you tick, after you review every step.</div>
<a class="btn" href="{RELEASES}">Download free</a></div>"""


def rule_page(r):
    rid, name = r["id"], r["name"]
    paths = [p for p in r.get("paths", [])]
    verdict, why, cls = VERDICT[r["safety"]]
    cmd = " ".join(r["official_cmd"]) if r.get("official_cmd") else None
    dry = " ".join(r["dry_run_cmd"]) if r.get("dry_run_cmd") else None
    fb = r.get("fallback", "none")
    manual = {
        "delete-contents": f"Delete what's inside the folder (quit related apps first).",
        "delete-dir": "Delete the folder.",
        "trash": "Move the folder to the Trash, so you can put it back if needed.",
        "delete-old": f"Delete files older than {r.get('older_than_days', 30)} days.",
    }.get(fb)
    measured = MEASURED.get(rid)
    title = f"What is {name} on Mac, and is it safe to delete?"
    desc = f"{name}: {r['explanation'].split('. ')[0]}. {verdict} Where it is, how big it gets, and the exact command to clean it."
    loc = "".join(f"<li><code>{e(p)}</code></li>" for p in paths) or "<li>Managed by macOS (see below).</li>"
    env = f"<p>Moved somewhere else? The <code>{e(r['env'])}</code> environment variable overrides the location.</p>" if r.get("env") else ""
    how = ""
    if cmd:
        how += f"<p>Use the tool's own command:</p><pre><code>{e(cmd)}</code></pre>"
        if dry:
            how += f"<p>Preview first with <code>{e(dry)}</code>.</p>"
        if manual and fb != "none":
            how += f"<p>If the tool isn't installed: {e(manual)}</p>"
    elif manual and r["safety"] != "never_auto":
        how += f"<p>{e(manual)}</p>"
    else:
        how += "<p>SpaceSense doesn't clean this automatically. Follow the explanation above.</p>"
    if r.get("quit_apps"):
        how += f"<p>Quit {e(', '.join(r['quit_apps']))} first.</p>"
    if r.get("leftover_explanation"):
        how += f"<p><b>If the app is uninstalled:</b> {e(r['leftover_explanation'])}</p>"
    docs = f'<p class="src">Source: <a href="{e(r["docs_url"])}">{e(r["docs_url"])}</a></p>' if r.get("docs_url") else ""
    body = f"""<article>
<p class="crumb"><a href="{BASE}guides/">Guides</a> › {e(CATEGORY_TITLE.get(r['category'], r['category']))}</p>
<h1>What is {e(name)} on Mac?</h1>
<div class="answer {cls}"><p class="verdict">{e(verdict)}</p><p>{e(why)}</p></div>
<p class="lead">{e(r['explanation'])}</p>
<h2>Where is it?</h2><ul>{loc}</ul>{env}
<h2>How big does it get?</h2>
<p>{('On the author’s Mac it was <b>' + e(measured) + '</b>.') if measured else 'It depends on how much you use it; SpaceSense measures it exactly.'}
Sizes count real disk use, like <code>du</code>.</p>
<h2>Does it come back?</h2><p>{e(r['comes_back'])}</p>
<h2>How do I clean it?</h2>{how}
{cta()}
{docs}
</article>"""
    faq = {
        "@context": "https://schema.org",
        "@type": "FAQPage",
        "mainEntity": [
            {"@type": "Question", "name": f"What is {name} on Mac?",
             "acceptedAnswer": {"@type": "Answer", "text": r["explanation"]}},
            {"@type": "Question", "name": f"Is it safe to delete {name}?",
             "acceptedAnswer": {"@type": "Answer", "text": f"{verdict} {why} {r['comes_back']}"}},
            {"@type": "Question", "name": f"How do I clean {name}?",
             "acceptedAnswer": {"@type": "Answer", "text": (f"Run `{cmd}`." if cmd else (manual or "See the explanation."))}},
        ],
    }
    return title, desc, body, faq


def main():
    rules = []
    for f in sorted((ROOT / "core" / "catalog").glob("*.toml")):
        with open(f, "rb") as fh:
            rules.append(tomllib.load(fh))
    rules = [r for r in rules if not r.get("generic")] + [r for r in rules if r.get("generic")]

    if OUT.exists():
        shutil.rmtree(OUT)
    (OUT / "guides").mkdir(parents=True)
    shutil.copy(ROOT / "site" / "style.css", OUT / "style.css")
    shutil.copytree(ROOT / "docs" / "screenshots", OUT / "screenshots")
    shutil.copy(ROOT / "site" / "icon.png", OUT / "icon.png")
    urls = ["", "guides/", "system-data.html", "node-modules.html"]

    by_cat = {}
    for r in rules:
        title, desc, body, faq = rule_page(r)
        (OUT / "guides" / f"{r['id']}.html").write_text(page(title, desc, body, f"guides/{r['id']}.html", faq))
        urls.append(f"guides/{r['id']}.html")
        by_cat.setdefault(r["category"], []).append(r)

    def guide_list():
        out = ""
        for cat, rs in by_cat.items():
            items = "".join(
                f'<li><a href="{BASE}guides/{r["id"]}.html">{e(r["name"])}</a>'
                f'<span class="tag {VERDICT[r["safety"]][2]}">{e(r["safety"].replace("_", " ").replace("never auto", "don’t delete"))}</span>'
                + (f'<span class="size">{e(MEASURED[r["id"]].split(" (")[0])}</span>' if r["id"] in MEASURED else "")
                + "</li>"
                for r in rs)
            out += f"<h3>{e(CATEGORY_TITLE.get(cat, cat))}</h3><ul class='guides'>{items}</ul>"
        out += f"<h3>Projects</h3><ul class='guides'><li><a href='{BASE}node-modules.html'>node_modules and other build folders</a><span class='tag ok'>regenerable</span><span class='size'>39.6 GB</span></li></ul>"
        return out

    (OUT / "guides" / "index.html").write_text(page(
        "Mac storage guides: what is it, and is it safe to delete?",
        "Plain answers for the caches, tool data and app data that fill up a Mac: what each one is, how big it gets, whether it comes back, and the exact command to clean it.",
        f"<h1>What is filling your Mac?</h1><p class='lead'>Every cache and data folder SpaceSense knows, with a plain answer: what it is, whether it’s safe to delete, and how to clean it.</p>{guide_list()}{cta()}",
        "guides/"))

    (OUT / "index.html").write_text(page(
        "SpaceSense: see what's filling your Mac, and clean it safely",
        "Free, open-source Mac app that explains what is filling your disk (node_modules, npm and Yarn caches, Docker, Xcode, AI models, app data) and cleans only what you choose, after you review every step.",
        f"""<section class="hero"><img src="{BASE}icon.png" alt="SpaceSense icon" width="96" height="96">
<h1>See what’s filling your Mac.<br>Clean it safely.</h1>
<p class="lead">SpaceSense explains every folder: what it is, which app owns it, and whether it’s safe to delete. It knows developer junk (node_modules, package caches, Docker, Xcode, AI models) and never deletes anything you haven’t reviewed.</p>
<p><a class="btn big" href="{RELEASES}">Download for Mac (free)</a> <a class="btn ghost big" href="{REPO}">Source on GitHub</a></p>
<p class="small">Version {e(VERSION)} · macOS 13+ · Apple Silicon and Intel · MIT license</p></section>
<img class="shot" src="{BASE}screenshots/overview.png" alt="SpaceSense overview: disk usage, what could be freed, and what uses space">
<section class="grid">
<div><h3>Explains, doesn’t scare</h3><p>Every item says what it is, who created it, whether it comes back, and what deleting costs.</p></div>
<div><h3>Developer-aware</h3><p>node_modules per project with git state, npm/Yarn/pnpm/Bun/pip/Cargo/Go/Homebrew caches, Docker, Xcode, Playwright, Ollama and Hugging Face.</p></div>
<div><h3>A person in the loop</h3><p>Nothing is preselected. You tick items, review every exact path and command, then confirm. Your files only go to the Trash.</p></div>
<div><h3>Fast and exact</h3><p>A parallel Rust scanner: 4 million files in about 20 s, with totals equal to <code>du</code>. Never downloads iCloud files.</p></div>
</section>
<img class="shot" src="{BASE}screenshots/folders.png" alt="Folders screen: each app's data with owner and verdict">
<h2>Guides</h2>{guide_list()}""",
        ""))

    system_data_body = f"""<article><h1>What is “System Data” on Mac, and how do I reduce it?</h1>
<div class="answer warn"><p class="verdict">You can’t delete “System Data” as one thing, but most of it is named folders you can clean.</p>
<p>It’s mostly caches, app data, virtual machine disks and logs. A few parts (swap, macOS updates, snapshots) are managed by macOS.</p></div>
<p class="lead">In System Settings → General → Storage, macOS shows a grey “System Data” bar and offers no way to manage it. It lumps together everything it doesn’t put in another category.</p>
<h2>What’s inside</h2><ul>
<li><b>App and developer caches</b> in <code>~/Library/Caches</code>: often tens of GB (<a href="{BASE}guides/yarn-v1-cache.html">Yarn</a>, <a href="{BASE}guides/homebrew-cache.html">Homebrew</a>, browsers).</li>
<li><b>App data</b> in <code>~/Library/Application Support</code> and <code>~/Library/Containers</code>: for example <a href="{BASE}guides/docker-desktop.html">Docker’s disk image</a>, <a href="{BASE}guides/claude-vm.html">the Claude app’s VM</a>, <a href="{BASE}guides/chrome-ai-model.html">Chrome’s AI model</a>.</li>
<li><b>Logs</b> in <code>~/Library/Logs</code> (<a href="{BASE}guides/logs.html">guide</a>).</li>
<li><b>Local Time Machine snapshots</b> (<a href="{BASE}guides/tm-snapshots.html">guide</a>) and <b>iPhone backups</b> (<a href="{BASE}guides/ios-backups.html">guide</a>).</li>
<li><b>Swap, sleep image and macOS update files</b> on macOS’s own volumes: macOS manages these; restarting often shrinks swap.</li></ul>
<h2>How to reduce it safely</h2><ol>
<li>Find the biggest folders. SpaceSense lists them with what they are; Finder doesn’t show folder sizes by default.</li>
<li>Clean caches with the tool’s own command (each guide has it), or delete them while the app is quit.</li>
<li>Move app data you don’t need to the Trash rather than deleting it, so you can put it back.</li>
<li>Leave Mail, Messages, Photos, Keychains and backups alone; manage them in their apps.</li></ol>
{cta()}</article>"""
    (OUT / "system-data.html").write_text(page(
        "What is System Data on Mac, and how do I reduce it?",
        "Mac “System Data” is mostly caches, app data, VM disks and logs. What’s inside it, what’s safe to delete, and how to shrink it without breaking anything.",
        system_data_body, "system-data.html",
        {"@context": "https://schema.org", "@type": "FAQPage", "mainEntity": [
            {"@type": "Question", "name": "What is System Data on Mac?", "acceptedAnswer": {"@type": "Answer", "text": "Everything macOS doesn't put in another storage category: app and developer caches, app data, virtual machine disks, logs, local Time Machine snapshots, iPhone backups, swap and macOS update files."}},
            {"@type": "Question", "name": "Can I delete System Data on Mac?", "acceptedAnswer": {"@type": "Answer", "text": "Not as one thing, but most of it is named folders (caches, app data, logs) that can be cleaned individually; swap and update files are managed by macOS."}},
        ]}))

    nm_body = f"""<article><h1>node_modules taking up space on Mac: what’s safe to delete</h1>
<div class="answer ok"><p class="verdict">Yes, node_modules is safe to delete in projects you’re not working on.</p>
<p>It’s rebuilt by <code>npm install</code>, <code>yarn</code>, <code>pnpm install</code> or <code>bun install</code>.</p></div>
<p class="lead">Every JavaScript project keeps its own copy of its dependencies in <code>node_modules</code>, often 200 MB–2 GB each. Old projects, clones and git worktrees add up fast: the author’s Mac had <b>39.6 GB in 110 folders</b>.</p>
<h2>What else is safe in a project</h2><ul>
<li><code>.next</code>, <code>.turbo</code>, <code>.parcel-cache</code>, <code>.nuxt</code>, <code>.svelte-kit</code>, <code>.expo</code>: build caches, rebuilt on the next build.</li>
<li><code>target/</code> (Rust, Maven), <code>build/</code> (Gradle, CMake), <code>.build</code> (Swift), <code>Pods</code> (CocoaPods), <code>__pycache__</code>, <code>.pytest_cache</code>.</li>
<li><code>dist/</code> and virtualenvs (<code>.venv</code>): usually safe, but look first.</li></ul>
<h2>Be careful with</h2><ul>
<li>Projects you’re actively working on (you’ll reinstall immediately).</li>
<li>A <code>node_modules</code> that git <i>doesn’t</i> ignore: it may be committed on purpose.</li>
<li><code>node_modules</code> inside <code>~/.nvm</code> or app bundles: those are installed tools, not project dependencies.</li></ul>
<h2>Find them all</h2><pre><code>find ~ -name node_modules -type d -prune -not -path '*/Library/*' -exec du -sh {{}} + | sort -h</code></pre>
<p>SpaceSense does this for you, with each project’s last activity and git state, and protects anything touched in the last 14 days.</p>
{cta()}</article>"""
    (OUT / "node-modules.html").write_text(page(
        "node_modules taking up space on Mac: what's safe to delete",
        "node_modules and build folders (.next, target, Pods, __pycache__) are safe to delete in projects you're not working on. How to find them all and what to be careful with.",
        nm_body, "node-modules.html"))

    (OUT / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
        + "".join(f"  <url><loc>{BASE}{u}</loc></url>\n" for u in urls) + "</urlset>\n")
    (OUT / "robots.txt").write_text(f"User-agent: *\nAllow: /\nSitemap: {BASE}sitemap.xml\n")
    (OUT / ".nojekyll").write_text("")
    print(f"built {len(urls)} pages in {OUT}")


if __name__ == "__main__":
    main()
