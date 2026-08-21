#!/bin/sh
# Cross-compile for the Kindle and stage device/ for a USB copy.
#
#   device/extensions/kfxdedrm-fe/  -> /mnt/us/extensions/kfxdedrm-fe/
#   device/documents/KFXDeDRM.sh    -> /mnt/us/documents/KFXDeDRM.sh
#
# One armv7 musl binary covers the KOA2, Colorsoft and Scribe. The cross link
# goes through rust-lld; see .cargo/config.toml.
#
# The kfxdedrm engine and the bokai converter are separate projects and neither
# is staged here; the app downloads them from their own GitHub releases.
set -eu

TARGET="armv7-unknown-linux-musleabihf"
CRATE="kfxdedrm-fe-native"
ROOT="$(cd "$(dirname "$0")" && pwd)"
OUT="$ROOT/device/extensions/kfxdedrm-fe/bin/kfxdedrm-fe"
# CARGO_TARGET_DIR carries a shared or cached build directory.
BUILT="${CARGO_TARGET_DIR:-$ROOT/target}/$TARGET"

# One line ahead of cargo's "can't find core for armv7-…" panic.
if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "error: rustup target '$TARGET' is not installed" >&2
    echo "       fix: rustup target add $TARGET" >&2
    exit 1
fi

# [workspace.package] holds the version; the binary reads it through
# CARGO_PKG_VERSION. VERSION covers the banner and the config.xml check.
VERSION="$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0}
                f && /^version *=/{gsub(/[" ]/,""); sub(/^version=/,""); print; exit}' \
    "$ROOT/Cargo.toml")"
[ -n "$VERSION" ] || { echo "error: no version in [workspace.package]" >&2; exit 1; }

echo "==> building kfxdedrm-fe $VERSION for $TARGET"
cargo build --release --target "$TARGET" -p "$CRATE" --bin "$CRATE"

# `kfxdedrm-fe` on device: launch.sh runs `pidof kfxdedrm-fe`. The cargo target
# keeps the longer name, which a host build cannot collide with.
mkdir -p "$(dirname "$OUT")"
cp "$BUILT/release/$CRATE" "$OUT"
chmod +x "$OUT" 2>/dev/null || true
chmod +x "$ROOT/device/extensions/kfxdedrm-fe/bin/launch.sh" 2>/dev/null || true

echo "==> staged $(ls -lh "$OUT" | awk '{print $5}') -> device/extensions/kfxdedrm-fe/bin/kfxdedrm-fe"
file "$OUT" 2>/dev/null || true

# KUAL reads the version from config.xml, which is written by hand.
CONFIG="$ROOT/device/extensions/kfxdedrm-fe/config.xml"
if ! grep -q "<version>$VERSION</version>" "$CONFIG" 2>/dev/null; then
    echo "warning: $CONFIG does not say <version>$VERSION</version>" >&2
fi

# The `# Icon:` line is a ~23KB base64 blob from device/make-tile.sh, which
# needs rsvg-convert and pngquant. Rewriting it per build churns that line into
# every diff, and TILE ships with the icon embedded. Checked, not regenerated.
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
Settings -> Add-ons fetches both over Wi-Fi, into /mnt/us/extensions/kfxdedrm/
and /mnt/us/extensions/bokai/, and the first screen offers the same when the
engine is missing. Either can still be unzipped there by hand.

Without bokai the two "Also write" settings do nothing and the app decrypts and
stops there. Decrypted books land in /mnt/us/dedrm/.
Logs, if anything goes wrong, in /mnt/us/logs/kfxdedrm-fe.log.
EOF
