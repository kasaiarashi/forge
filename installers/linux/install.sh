#!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
CONFIG_DIR="${CONFIG_DIR:-/etc/forge}"
DATA_DIR="${DATA_DIR:-/var/lib/forge}"
REPO="${FORGE_REPO:-kasaiarashi/forge}"
VERSION="${FORGE_VERSION:-}"

SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo /tmp)"

# ── Bootstrap mode ─────────────────────────────────────────────────────
#
# This same script ships two ways:
#
#   1. Inside the release tarball, next to forge-server / forge-web /
#      ui / forge-server.toml / forge-web.toml. The "in-tarball" path
#      below installs from those sibling files.
#
#   2. Hosted on raw.githubusercontent.com so users can do:
#
#        curl -fsSL https://raw.githubusercontent.com/kasaiarashi/forge/master/installers/linux/install.sh | sudo bash
#
#      In that mode there are no sibling files — the script needs to
#      download the tarball itself, extract it to a tmpdir, and
#      re-exec from inside.
#
# We tell the two modes apart by looking for `forge-server` AND
# `forge-server.toml` next to the script (only true inside the tarball;
# never true in /usr/bin or wherever curl-pipe might have dropped the
# script). When the bootstrap path runs, it downloads the latest (or
# `FORGE_VERSION`-pinned) release tarball and re-execs the in-tarball
# install.sh with the same env vars.
if [ ! -f "$SCRIPT_DIR/forge-server" ] || [ ! -f "$SCRIPT_DIR/forge-server.toml" ]; then
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) asset="forge-server-linux-amd64.tar.gz" ;;
        *)
            echo "Unsupported architecture: $arch" >&2
            echo "Forge currently publishes Linux binaries for x86_64 only." >&2
            exit 1
            ;;
    esac

    if [ -z "$VERSION" ]; then
        echo "Resolving latest Forge release..."
        VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
            | grep -m1 '"tag_name"' \
            | sed -E 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/')"
        if [ -z "$VERSION" ]; then
            echo "Failed to resolve latest release tag from GitHub API." >&2
            echo "Set FORGE_VERSION=vX.Y.Z manually and re-run." >&2
            exit 1
        fi
    fi

    url="https://github.com/$REPO/releases/download/$VERSION/$asset"
    echo "Downloading $url"

    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    if ! curl -fsSL "$url" -o "$tmp/$asset"; then
        echo "Download failed. Check the version ($VERSION) exists." >&2
        exit 1
    fi
    tar xzf "$tmp/$asset" -C "$tmp"

    extracted="$tmp/${asset%.tar.gz}"
    if [ ! -f "$extracted/install.sh" ]; then
        echo "Tarball $asset does not contain install.sh" >&2
        exit 1
    fi

    # Hand off to the in-tarball copy of this script. `exec` replaces
    # the current process so the trap above also drops out — but the
    # tmpdir is what we need to keep around until install.sh is done,
    # so we let install.sh inherit the trap by running it as a child
    # rather than exec'ing.
    PREFIX="$PREFIX" CONFIG_DIR="$CONFIG_DIR" DATA_DIR="$DATA_DIR" \
        bash "$extracted/install.sh"
    exit $?
fi

echo "Installing Forge VCS Server..."
echo "  PREFIX:     $PREFIX"
echo "  CONFIG_DIR: $CONFIG_DIR"
echo "  DATA_DIR:   $DATA_DIR"
echo ""

# Install binaries
echo "Installing binaries to $PREFIX/bin/..."
install -d "$PREFIX/bin"
install -m 755 "$SCRIPT_DIR/forge-server" "$PREFIX/bin/forge-server"
install -m 755 "$SCRIPT_DIR/forge-web"    "$PREFIX/bin/forge-web"

# Install web UI assets
echo "Installing web UI to $PREFIX/share/forge/ui/..."
install -d "$PREFIX/share/forge/ui"
cp -r "$SCRIPT_DIR/ui/"* "$PREFIX/share/forge/ui/"

# Install config templates (don't overwrite existing)
echo "Installing config templates to $CONFIG_DIR/..."
install -d "$CONFIG_DIR"
if [ ! -f "$CONFIG_DIR/forge-server.toml" ]; then
    install -m 644 "$SCRIPT_DIR/forge-server.toml" "$CONFIG_DIR/forge-server.toml"
    # Update paths to match installed locations
    sed -i "s|base_path = \"./forge-data\"|base_path = \"$DATA_DIR\"|" "$CONFIG_DIR/forge-server.toml"
else
    echo "  Skipping forge-server.toml (already exists)"
fi

if [ ! -f "$CONFIG_DIR/forge-web.toml" ]; then
    install -m 644 "$SCRIPT_DIR/forge-web.toml" "$CONFIG_DIR/forge-web.toml"
    sed -i "s|static_dir = \"./ui\"|static_dir = \"$PREFIX/share/forge/ui\"|" "$CONFIG_DIR/forge-web.toml"
else
    echo "  Skipping forge-web.toml (already exists)"
fi

# Create data directory
echo "Creating data directory at $DATA_DIR/..."
install -d "$DATA_DIR"

echo ""
echo "Forge VCS Server installed successfully!"
echo ""
echo "  Binaries: $PREFIX/bin/forge-server, $PREFIX/bin/forge-web"
echo "  Config:   $CONFIG_DIR/forge-server.toml, $CONFIG_DIR/forge-web.toml"
echo "  Data:     $DATA_DIR/"
echo "  Web UI:   $PREFIX/share/forge/ui/"
echo ""
echo "Start the server:"
echo "  forge-server --config $CONFIG_DIR/forge-server.toml"
echo ""
echo "Start the web UI:"
echo "  forge-web --config $CONFIG_DIR/forge-web.toml"
