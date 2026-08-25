#!/bin/sh
# Rewrites the settings fixtures both suites read. See regen-fixtures.lua.
#   koplugin/spec/regen-fixtures.sh
# Then read the diff, and run both suites:
#   koplugin/spec/run.sh ports_spec
#   cargo test -p kfxdedrm-fe-native --test shared_settings_file
set -eu

SPEC="$(cd "$(dirname "$0")" && pwd)"
export KFXDEDRM_SPEC="$SPEC"
export LUA_PATH="$SPEC/?.lua;;"

command -v luajit >/dev/null 2>&1 || {
    echo "error: luajit is not installed" >&2
    exit 1
}

luajit "$SPEC/regen-fixtures.lua"
