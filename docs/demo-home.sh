#!/bin/sh
# Builds a fake home folder with realistic developer clutter, for screenshots.
# Sizes come from APFS clones of one 256 MB file: each clone is counted at full
# size (like `du`), but they share their blocks, so the demo uses ~256 MB of disk.
#
# usage: docs/demo-home.sh DIR      (DIR must be on an APFS volume)
set -e
H="${1:?usage: demo-home.sh DIR}"
rm -rf "$H"
mkdir -p "$H"
BASE="$H/.seed"
mkdir -p "$BASE"
# Random data (not zeros) so APFS can't make it sparse.
head -c 268435456 /dev/urandom > "$BASE/blob"

# fill DIR NAME_PREFIX COUNT  →  COUNT × 256 MB in DIR
fill() {
    mkdir -p "$1"
    i=0
    while [ $i -lt "$3" ]; do
        cp -c "$BASE/blob" "$1/$2$i"
        i=$((i + 1))
    done
}
# small DIR FILE BYTES
small() { mkdir -p "$1"; head -c "$3" /dev/urandom > "$1/$2"; }
age() { find "$1" -exec touch -h -t "$2" {} + ; }

# Package caches
fill "$H/Library/Caches/Yarn/v6" npm-pkg- 84            # 21.5 GB
fill "$H/.npm/_cacache/content-v2" sha512- 44           # 11.3 GB
fill "$H/.npm/_npx/4f2a/node_modules" pkg- 4
fill "$H/Library/Caches/Homebrew/downloads" bottle- 5
fill "$H/Library/Caches/pip/wheels" whl- 2
fill "$H/.cargo/registry/cache" crate- 3
fill "$H/Library/Caches/ms-playwright/chromium-1140" chrome- 2
# Docker Desktop disk image (Docker itself isn't installed: a leftover)
fill "$H/Library/Containers/com.docker.docker/Data/vms/0/data" Docker.raw.part- 72
# App data
fill "$H/Library/Application Support/Google/Chrome/OptGuideOnDeviceModel/2025.8.21.1028" weights- 16
fill "$H/Library/Application Support/Google/Chrome/OptGuideOnDeviceModel/2025.8.8.1141" weights- 16
fill "$H/Library/Application Support/Google/Chrome/Default" history- 6
fill "$H/Library/Application Support/Claude/vm_bundles/claudevm.bundle" rootfs- 40
fill "$H/Library/Application Support/Slack" cache- 4
fill "$H/Library/Application Support/Notion" data- 8
fill "$H/Library/Application Support/Code/User" state- 3
fill "$H/Library/Caches/com.spotify.client" storage- 2
fill "$H/Library/Caches/Google/Chrome" cache- 6
fill "$H/Library/Caches/com.tinyspeck.slackmacgap" cache- 2
fill "$H/Library/Containers/com.tinyspeck.slackmacgap/Data" d- 3
fill "$H/Library/Logs/DiagnosticReports" crash- 1
age "$H/Library/Logs" 202501010000
small "$H/Library/Preferences" com.apple.finder.plist 40000
small "$H/Library/Keychains" login.keychain-db 300000
small "$H/.ssh" id_ed25519 400

# Projects: name, kind, size (×256 MB), age (touch -t), artifacts
project() {
    p="$H/code/$1"
    mkdir -p "$p/src"
    echo "export const x = 1" > "$p/src/index.ts"
    case "$2" in
        node) echo '{"name":"'"$1"'"}' > "$p/package.json"
              fill "$p/node_modules/.pnpm" dep- "$3"
              [ -n "$5" ] && fill "$p/$5" chunk- 2
              printf 'node_modules/\n.next/\ndist/\n' > "$p/.gitignore" ;;
        rust) printf '[package]\nname = "%s"\n' "$1" > "$p/Cargo.toml"
              fill "$p/target/debug/deps" lib- "$3"
              printf 'target/\n' > "$p/.gitignore" ;;
        py)   echo "torch" > "$p/requirements.txt"
              fill "$p/.venv/lib/python3.12/site-packages" whl- "$3"
              mkdir -p "$p/__pycache__" && echo x > "$p/__pycache__/m.pyc"
              printf '.venv/\n__pycache__/\n' > "$p/.gitignore" ;;
    esac
    (cd "$p" && git init -q -b main && git add -A . 2>/dev/null; git -c user.name=demo -c user.email=demo@example.com commit -qm init >/dev/null 2>&1 || true)
    find "$p" -path "$p/.git" -prune -o -exec touch -h -t "$4" {} +
}
NOW=$(date +%Y%m%d%H%M)
project storefront      node 10 "$NOW"        .next
project api-server      node 5  202604150900
project old-portfolio   node 4  202303011200  .next
project marketing-site  node 6  202501101000  .next
project image-cli       rust 24 202609010900
project ml-experiments  py   7  202407200900
project chat-widget     node 3  202211051500  dist
# A git worktree of storefront (shares its repo)
(cd "$H/code/storefront" && git worktree add -q ../storefront-checkout -b feature/checkout 2>/dev/null || true)
fill "$H/code/storefront-checkout/node_modules/.pnpm" dep- 9

# Your files: one real 512 MB file, cloned (large files are listed from 500 MB).
cat "$BASE/blob" "$BASE/blob" > "$BASE/big"
mkdir -p "$H/Downloads" "$H/Movies"
for f in "Downloads/Xcode_16.2.xip" "Downloads/ubuntu-24.04-desktop-arm64.iso" \
         "Downloads/old-backup-2023.zip" "Movies/Product demo (final).mov" "Movies/Conference talk.mp4"; do
    cp -c "$BASE/big" "$H/$f"
done
fill "$H/Documents/Design" mockups- 3
fill "$H/Pictures/Photos Library.photoslibrary" originals- 12
fill "$H/Desktop/Screenshots" shots- 2
age "$H/Downloads" 202502101200

rm -rf "$BASE"
du -sh "$H" 2>/dev/null | awk '{print "demo home (counted like du):", $1}'
