#!/bin/bash
set -e

BINARY="cathode"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

# --- detect architecture ---

ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)       PRETTY="x86_64"  ;;
    aarch64)      PRETTY="aarch64" ;;
    armv7l|armv7) PRETTY="armv7h"  ;;
    *)            PRETTY="$ARCH"   ;;
esac

echo "=== cathode installer ==="
echo ""
echo "  arch:    $PRETTY ($ARCH)"
echo "  host:    $(cat /etc/hostname 2>/dev/null || echo unknown)"
echo "  install: $INSTALL_DIR/$BINARY"
echo ""

# --- install runtime deps ---

DEPS=(yt-dlp mpv curl)
MISSING=()

for dep in "${DEPS[@]}"; do
    if ! pacman -Qi "$dep" &>/dev/null; then
        MISSING+=("$dep")
    fi
done

if [ ${#MISSING[@]} -gt 0 ]; then
    echo "[1/4] Installing runtime deps: ${MISSING[*]}"
    sudo pacman -S --needed --noconfirm "${MISSING[@]}"
else
    echo "[1/4] Runtime deps present (${DEPS[*]})"
fi

# --- ensure rust toolchain ---

if ! command -v cargo &>/dev/null; then
    echo "[2/4] Installing rust toolchain..."
    if pacman -Ss '^rust$' &>/dev/null; then
        sudo pacman -S --needed --noconfirm rust
    else
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        source "$HOME/.cargo/env"
    fi
else
    RUST_VER="$(rustc --version | awk '{print $2}')"
    echo "[2/4] Rust $RUST_VER present"
fi

# --- build ---

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "[3/4] Building cathode (release, native)..."
RUSTFLAGS="-C target-cpu=native" cargo build --release 2>&1

BUILT="target/release/$BINARY"
SIZE="$(du -h "$BUILT" | cut -f1)"
echo "      Built: $BUILT ($SIZE)"

# --- install ---

mkdir -p "$INSTALL_DIR"
rm -f "$INSTALL_DIR/$BINARY"
cp "$BUILT" "$INSTALL_DIR/$BINARY"
chmod +x "$INSTALL_DIR/$BINARY"
echo "[4/5] Installed to $INSTALL_DIR/$BINARY"

# --- configure resolution ---

CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/cathode"
CONFIG_FILE="$CONFIG_DIR/config.toml"

# detect monitor height: try wlr-randr, then xrandr, then /sys
detect_resolution() {
    # wayland
    if command -v wlr-randr &>/dev/null; then
        wlr-randr 2>/dev/null | grep -oP '\d+x\d+' | head -1 | cut -d'x' -f2 && return
    fi
    # x11
    if command -v xrandr &>/dev/null; then
        xrandr 2>/dev/null | grep '\*' | head -1 | grep -oP '\d+x\d+' | cut -d'x' -f2 && return
    fi
    # fallback: drm
    for f in /sys/class/drm/*/modes; do
        if [ -f "$f" ]; then
            head -1 "$f" | cut -d'x' -f2 && return
        fi
    done
    echo ""
}

DETECTED_RES="$(detect_resolution)"

if [ -n "$DETECTED_RES" ] && [ "$DETECTED_RES" -gt 0 ] 2>/dev/null; then
    # snap to nearest standard
    if   [ "$DETECTED_RES" -ge 2160 ]; then MAX_RES=2160
    elif [ "$DETECTED_RES" -ge 1440 ]; then MAX_RES=1440
    elif [ "$DETECTED_RES" -ge 1080 ]; then MAX_RES=1080
    elif [ "$DETECTED_RES" -ge 720 ];  then MAX_RES=720
    else MAX_RES=480
    fi
    echo "[5/5] Detected display: ${DETECTED_RES}p -> max_resolution = ${MAX_RES}"
else
    MAX_RES=1080
    echo "[5/5] Could not detect display, defaulting to max_resolution = ${MAX_RES}"
fi

# write config only if it doesn't exist (don't clobber user edits)
if [ ! -f "$CONFIG_FILE" ]; then
    mkdir -p "$CONFIG_DIR"
    cat > "$CONFIG_FILE" << EOF
# cathode configuration
# detected during install — edit freely

max_resolution = $MAX_RES
region = "US"
feed_sample = 4
EOF
    echo "      Wrote $CONFIG_FILE"
else
    echo "      Config exists, not overwriting ($CONFIG_FILE)"
fi

# check PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo ""
       echo "  NOTE: $INSTALL_DIR is not in PATH"
       echo "  Add to your shell rc:  export PATH=\"$INSTALL_DIR:\$PATH\""
       ;;
esac

echo ""
echo "=== done ==="
