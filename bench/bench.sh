#!/bin/zsh
# M0 benchmark: SpaceSense vs du / dust / diskus on one root (default: $HOME).
# Warm-cache numbers by default. For cold numbers run `sudo purge` before each
# tool (pass --cold to be prompted for it between runs).
#
# usage: bench/bench.sh [ROOT] [--cold]
set -u
ROOT=${1:-$HOME}
COLD=${2:-}
SS=${SS:-$(dirname $0)/../target/release/spacesense}

run() {   # name, command...
  local name=$1; shift
  [[ $COLD == --cold ]] && { echo "purging caches for $name"; sudo purge; }
  local t0=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  local out=$("$@" 2>/dev/null | tail -1)
  local t1=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  printf "%-22s %8.2fs   %s\n" $name $(( t1 - t0 )) "$out"
}

echo "root: $ROOT   ($(sysctl -n hw.ncpu) CPUs, $(sw_vers -productVersion))"
run "spacesense (bulk)"  $SS scan $ROOT --kib
run "spacesense (stat)"  $SS scan $ROOT --kib --backend stat
run "du -skx"            du -skx $ROOT
command -v diskus >/dev/null && run "diskus"  diskus $ROOT
command -v dust   >/dev/null && run "dust -d0 -x"  dust -d 0 -x -n 1 -P -b $ROOT
