#!/bin/sh
# The launch path for both documents/KFXDeDRM.sh and menu.json.

BIN_DIR=/mnt/us/extensions/kfxdedrm-fe/bin
# One binary per float ABI, hard-float first: on a device that starts both, it
# is the one to run. Word-split into arguments below, so no quotes around it.
VARIANTS="kfxdedrm-fe kfxdedrm-fe-armsf"
LOG=/mnt/us/logs/kfxdedrm-fe.log

# Read before launch: peekHistoryView names the view this launch pushes onto.
APPMGR=com.lab126.appmgrd
ORIGIN_VIEW="$(lipc-get-prop "$APPMGR" peekHistoryView 2>/dev/null)"

# A second launch over a running instance leaves the framework SIGSTOP'd and
# the screen frozen. Both names, because either build may be the running one.
# The engine's own processes are named `kfxdedrm`, which neither matches.
# shellcheck disable=SC2086
if pidof $VARIANTS >/dev/null 2>&1; then
    exit 0
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

# A stock user partition carries no /mnt/us/logs.
mkdir -p "$(dirname "$LOG")"

# `--version` draws nothing and exits 0, so the first variant that answers it
# is the one this device's loader accepts. A binary that is absent or built for
# the other ABI fails here rather than on a blank screen.
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
