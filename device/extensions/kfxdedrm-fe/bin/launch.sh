#!/bin/sh
# The launch path for both documents/KFXDeDRM.sh and menu.json.

BIN_DIR=/mnt/us/extensions/kfxdedrm-fe/bin
# One binary per float ABI, hard-float first: on a device that starts both, it
# is the one to run. Word-split into arguments below; no quotes around it.
VARIANTS="kfxdedrm-fe kfxdedrm-fe-armsf"
LOG=/mnt/us/logs/kfxdedrm-fe.log

# Read before launch: peekHistoryView names the view this launch pushes onto.
APPMGR=com.lab126.appmgrd
ORIGIN_VIEW="$(lipc-get-prop "$APPMGR" peekHistoryView 2>/dev/null)"

# A second launch over a running instance leaves the framework SIGSTOP'd and
# the screen frozen. Either build may be the running one, hence both names.
# The engine's own processes are named `kfxdedrm`, which neither matches.
# shellcheck disable=SC2086
if pidof $VARIANTS >/dev/null 2>&1; then
    exit 0
fi

# A stock user partition carries no /mnt/us/logs.
mkdir -p "$(dirname "$LOG")"

# The tree the last update replaced. This run reads none of it.
rm -rf /mnt/us/extensions/kfxdedrm-fe.old

# An update staged by Settings, which `install::selfupdate` proved starts on
# this device. The move happens here, with nothing under the folder open, and
# the new script takes the launch from there.
if [ -f /mnt/us/extensions/kfxdedrm-fe.new/bin/launch.sh ]; then
    # Both frontends write these two inside the folder, either of them after
    # `install::selfupdate` staged its copy. The live one wins.
    for state in config.txt installs.txt; do
        if [ -f "/mnt/us/extensions/kfxdedrm-fe/$state" ]; then
            cp "/mnt/us/extensions/kfxdedrm-fe/$state" \
                "/mnt/us/extensions/kfxdedrm-fe.new/$state" 2>> "$LOG"
        fi
    done
    if mv /mnt/us/extensions/kfxdedrm-fe /mnt/us/extensions/kfxdedrm-fe.old 2>> "$LOG" &&
        mv /mnt/us/extensions/kfxdedrm-fe.new /mnt/us/extensions/kfxdedrm-fe 2>> "$LOG"; then
        echo "[$(date)] update applied" >> "$LOG"
        exec /mnt/us/extensions/kfxdedrm-fe/bin/launch.sh
    fi
    # Half-moved: the old tree back, and the staged copy left for a retry.
    if [ ! -d /mnt/us/extensions/kfxdedrm-fe ]; then
        mv /mnt/us/extensions/kfxdedrm-fe.old /mnt/us/extensions/kfxdedrm-fe 2>> "$LOG"
    fi
    echo "[$(date)] update staged but not applied" >> "$LOG"
fi

# Armed past the guard: setting startView while the app holds the screen pulls
# the view out from under it.
restore_view_on_exit() {
    case "$ORIGIN_VIEW" in
        KPP_*|LEGACY_*)
            lipc-set-prop "$APPMGR" startView \
                "$ORIGIN_VIEW:0:app://com.lab126.KPPMainApp?view=$ORIGIN_VIEW" 2>/dev/null
            ;;
    esac
}
trap restore_view_on_exit EXIT

# `--version` draws nothing and exits 0. The first variant answering it is the
# one this device's loader accepts; a binary that is absent or built for the
# other ABI fails here, ahead of a blank screen.
BIN=
for name in $VARIANTS; do
    if "$BIN_DIR/$name" --version >/dev/null 2>&1; then
        BIN="$BIN_DIR/$name"
        break
    fi
done

if [ -z "$BIN" ]; then
    echo "[$(date)] no build in $BIN_DIR runs on $(uname -m)" >> "$LOG"
    exit 1
fi

echo "[$(date)] launch ${BIN##*/} $(uname -m)" >> "$LOG"
"$BIN" 2>> "$LOG"
# Own line: the `$(date)` below overwrites `$?` in some shells.
STATUS=$?
echo "[$(date)] exit=$STATUS" >> "$LOG"
