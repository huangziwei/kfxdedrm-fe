#!/bin/sh
# The plugin's suite, on a host. Needs luajit and, for unpack_spec, unzip.
#
#   koplugin/spec/run.sh              every spec
#   koplugin/spec/run.sh ports_spec   one of them
#
# unpack_spec runs Install.unpack over the two archives the plugin downloads on
# device. They are fetched into spec/cache/ once and are not in the repository;
# without a network that one spec is skipped and the rest still run.
set -eu

SPEC="$(cd "$(dirname "$0")" && pwd)"
export KFXDEDRM_SPEC="$SPEC"
# harness.lua is what puts the plugin on package.path, so it has to be findable
# before it runs. The trailing `;;` keeps luajit's own default path.
export LUA_PATH="$SPEC/?.lua;;"
CACHE="$SPEC/cache"

command -v luajit >/dev/null 2>&1 || {
    echo "error: luajit is not installed" >&2
    exit 1
}

# The releases the two archives come from, pinned: the spec asserts their sizes.
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

rm -rf "$CACHE/tree" "$CACHE/unpacked"
exit $status
