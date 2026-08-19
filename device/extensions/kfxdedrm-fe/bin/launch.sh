#!/bin/sh
# The launch path for both documents/KFXDeDRM.sh and menu.json.

BIN=/mnt/us/extensions/kfxdedrm-fe/bin/kfxdedrm-fe
LOG=/mnt/us/logs/kfxdedrm-fe.log

# Read before launch: peekHistoryView names the view this launch pushes onto.
APPMGR=com.lab126.appmgrd
ORIGIN_VIEW="$(lipc-get-prop "$APPMGR" peekHistoryView 2>/dev/null)"

# A second launch over a running instance leaves the framework SIGSTOP'd and
# the screen frozen. The engine's own processes are named `kfxdedrm`, which
# `pidof kfxdedrm-fe` does not match.
if pidof kfxdedrm-fe >/dev/null 2>&1; then
    exit 0
fi

# Armed past the guard: setting startView while BIN holds the screen pulls the
# view out from under it.
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

echo "[$(date)] launch $(uname -m)" >> "$LOG"
"$BIN" 2>> "$LOG"
# Own line: the `$(date)` below overwrites `$?` in some shells.
STATUS=$?
echo "[$(date)] exit=$STATUS" >> "$LOG"
