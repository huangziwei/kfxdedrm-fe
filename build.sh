#!/bin/sh
# Cross-compile and stage device/ for a USB copy to /mnt/us.
#   ./build.sh          armv7-unknown-linux-musleabihf, bin/kfxdedrm-fe
#   ./build.sh armsf    armv7-unknown-linux-musleabi, bin/kfxdedrm-fe-armsf
set -eu

CRATE="kfxdedrm-fe-native"
ROOT="$(cd "$(dirname "$0")" && pwd)"

ABI="${1:-armhf}"
case "$ABI" in
armhf)
    TARGET="armv7-unknown-linux-musleabihf"
    NAME="kfxdedrm-fe"
    # WANT_FLOAT is the e_flags byte at 0x25: 04 hardfloat, 02 soft-float.
    WANT_FLOAT="04"
    ;;
armsf)
    TARGET="armv7-unknown-linux-musleabi"
    NAME="kfxdedrm-fe-armsf"
    WANT_FLOAT="02"
    ;;
*)
    echo "usage: $0 [armhf|armsf]" >&2
    exit 1
    ;;
esac

OUT="$ROOT/device/extensions/kfxdedrm-fe/bin/$NAME"
# CARGO_TARGET_DIR carries a shared or cached build directory.
BUILT="${CARGO_TARGET_DIR:-$ROOT/target}/$TARGET"

# One line ahead of cargo's "can't find core for armv7-…" panic.
if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "error: rustup target '$TARGET' is not installed" >&2
    echo "       fix: rustup target add $TARGET" >&2
    exit 1
fi

# [workspace.package] is the one place the version is edited.
VERSION="$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0}
                f && /^version *=/{gsub(/[" ]/,""); sub(/^version=/,""); print; exit}' \
    "$ROOT/Cargo.toml")"
[ -n "$VERSION" ] || { echo "error: no version in [workspace.package]" >&2; exit 1; }

echo "==> building kfxdedrm-fe $VERSION for $TARGET"
cargo build --release --target "$TARGET" -p "$CRATE" --bin "$CRATE"

# launch.sh runs `pidof` over these names.
mkdir -p "$(dirname "$OUT")"
cp "$BUILT/release/$CRATE" "$OUT"
chmod +x "$OUT" 2>/dev/null || true
chmod +x "$ROOT/device/extensions/kfxdedrm-fe/bin/launch.sh" 2>/dev/null || true

# e_machine at 0x12 (2800 = EM_ARM) and the e_flags float byte at 0x25.
MACHINE="$(od -An -tx1 -j18 -N2 "$OUT" | tr -d ' \n')"
FLOAT="$(od -An -tx1 -j37 -N1 "$OUT" | tr -d ' \n')"
[ "$MACHINE" = "2800" ] || {
    echo "error: $OUT is not an ARM ELF (e_machine 0x$MACHINE) — check .cargo/config.toml" >&2
    exit 1
}
[ "$FLOAT" = "$WANT_FLOAT" ] || {
    echo "error: $OUT is not $ABI (ELF float ABI byte 0x$FLOAT, wanted 0x$WANT_FLOAT)" >&2
    exit 1
}

echo "==> staged $(ls -lh "$OUT" | awk '{print $5}') -> device/extensions/kfxdedrm-fe/bin/$NAME"
file "$OUT" 2>/dev/null || true

# `want` into `file`, replacing whatever `match` finds there.
stamp() {
    file="$1"
    match="$2"
    want="$3"
    if [ ! -f "$file" ] || ! grep -q "$match" "$file"; then
        echo "warning: $file carries no version line to stamp" >&2
        return 0
    fi
    sed "s|$match|$want|" "$file" > "$file.stamp"
    if cmp -s "$file" "$file.stamp"; then
        rm -f "$file.stamp"
    else
        mv "$file.stamp" "$file"
        echo "==> stamped $VERSION into ${file#"$ROOT/"}"
    fi
}

# KUAL reads this one and shows it in the extension list.
stamp "$ROOT/device/extensions/kfxdedrm-fe/config.xml" \
    '<version>[^<]*</version>' "<version>$VERSION</version>"

# _meta.lua names this build in KOReader's own screen.
stamp "$ROOT/koplugin/kfxdedrm.koplugin/_meta.lua" \
    '^\( *\)version = "[^"]*",' "\1version = \"$VERSION\","

# The `# Icon:` line comes from device/make-tile.sh. Checked, not regenerated.
TILE="$ROOT/device/documents/KFXDeDRM.sh"
COVER="$ROOT/device/assets/cover.png"
if ! grep -q '^# Icon: data:image/png;base64,' "$TILE" 2>/dev/null; then
    echo "warning: $TILE has no embedded cover — run device/make-tile.sh" >&2
elif [ -f "$COVER" ] && [ "$COVER" -nt "$TILE" ]; then
    echo "warning: assets/cover.png is newer than the tile's embedded icon" >&2
    echo "         the old cover would ship — run device/make-tile.sh" >&2
fi

cat <<'EOF'

==> install — copy these two onto the device

    device/extensions/kfxdedrm-fe/  ->  /mnt/us/extensions/kfxdedrm-fe/
    device/documents/KFXDeDRM.sh    ->  /mnt/us/documents/KFXDeDRM.sh

Neither the kfxdedrm engine nor the optional bokai converter is staged here.
Settings -> Updates -> "kfxdedrm + bokai" fetches both over Wi-Fi, into
/mnt/us/extensions/kfxdedrm/ and /mnt/us/extensions/bokai/, and the first
screen offers the same when the engine is missing. Either can still be
unzipped there by hand.

Without bokai the two "Also write" settings do nothing and the app decrypts and
stops there. Decrypted books land in /mnt/us/dedrm/.
Logs, if anything goes wrong, in /mnt/us/logs/kfxdedrm-fe.log.
EOF
