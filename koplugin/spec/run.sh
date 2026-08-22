#!/bin/sh
# The plugin's suite, on a host. Needs luajit, and unzip for unpack_spec.
#
#   koplugin/spec/run.sh              every spec
#   koplugin/spec/run.sh ports_spec   one of them
set -eu

SPEC="$(cd "$(dirname "$0")" && pwd)"
export KFXDEDRM_SPEC="$SPEC"
# $SPEC holds harness.lua; the trailing `;;` keeps luajit's default path.
export LUA_PATH="$SPEC/?.lua;;"
# Scratch: the two archives below, and what the specs write.
CACHE="$SPEC/cache"
mkdir -p "$CACHE"

command -v luajit >/dev/null 2>&1 || {
    echo "error: luajit is not installed" >&2
    exit 1
}

# Pinned releases: unpack_spec asserts the sizes these two carry.
ENGINE_ZIP="https://github.com/Satsuoni/DeDRM_tools/releases/download/v10.0.30/kfxdedrmmobi.zip"
BOKAI_ZIP="https://github.com/huangziwei/sidle/releases/download/v0.1.9/bokai-v0.1.2-kindle.zip"

fetch() {
    [ -f "$2" ] && return 0
    mkdir -p "$CACHE"
    curl -sfL "$1" -o "$2" || { rm -f "$2"; return 1; }
}

have_archives=yes
fetch "$ENGINE_ZIP" "$CACHE/kfxdedrmmobi.zip" || have_archives=no
fetch "$BOKAI_ZIP" "$CACHE/bokai.zip" || have_archives=no

if [ $# -gt 0 ]; then
    specs="$*"
else
    specs="ports_spec main_spec install_spec unpack_spec"
fi

status=0
for spec in $specs; do
    if [ "$spec" = "unpack_spec" ] && [ "$have_archives" = "no" ]; then
        echo "unpack_spec    skipped -- the archives could not be fetched"
        continue
    fi
    printf '%-14s ' "$spec"
    luajit "$SPEC/$spec.lua" || status=1
done

rm -rf "$CACHE/tree" "$CACHE/unpacked" "$CACHE/abi"
exit $status
