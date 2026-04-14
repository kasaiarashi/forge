#!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
CONFIG_DIR="${CONFIG_DIR:-/usr/local/etc/forge}"
DATA_DIR="${DATA_DIR:-/usr/local/var/forge}"
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
#        curl -fsSL https://raw.githubusercontent.com/kasaiarashi/forge/master/installers/macos/install.sh | sudo bash
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
        x86_64|amd64) asset="forge-server-macos-amd64.tar.gz" ;;
        arm64)        asset="forge-server-macos-arm64.tar.gz" ;;
        *)
            echo "Unsupported architecture: $arch" >&2
            echo "Forge publishes macOS binaries for x86_64 and arm64." >&2
            exit 1
            ;;
    esac

    if [ -z "$VERSION" ]; then
        echo "Resolving latest Forge release..."
        if ! release_json="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")"; then
            echo "Failed to reach GitHub API for latest release." >&2
            echo "Set FORGE_VERSION=vX.Y.Z manually and re-run." >&2
            exit 1
        fi
        VERSION="$(grep -m1 '"tag_name"' <<< "$release_json" \
            | sed -E 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/')"
        if [ -z "$VERSION" ]; then
            echo "Failed to parse latest release tag from GitHub API response." >&2
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

    chmod +x "$extracted/install.sh"

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
mkdir -p "$PREFIX/bin"
cp "$SCRIPT_DIR/forge-server" "$PREFIX/bin/forge-server"
cp "$SCRIPT_DIR/forge-web"    "$PREFIX/bin/forge-web"
chmod 755 "$PREFIX/bin/forge-server" "$PREFIX/bin/forge-web"

# Install web UI assets
echo "Installing web UI to $PREFIX/share/forge/ui/..."
mkdir -p "$PREFIX/share/forge/ui"
cp -R "$SCRIPT_DIR/ui/"* "$PREFIX/share/forge/ui/"

# Install config templates (don't overwrite existing)
echo "Installing config templates to $CONFIG_DIR/..."
mkdir -p "$CONFIG_DIR"
if [ ! -f "$CONFIG_DIR/forge-server.toml" ]; then
    cp "$SCRIPT_DIR/forge-server.toml" "$CONFIG_DIR/forge-server.toml"
    sed -i '' "s|base_path = \"./forge-data\"|base_path = \"$DATA_DIR\"|" "$CONFIG_DIR/forge-server.toml"
else
    echo "  Skipping forge-server.toml (already exists)"
fi

if [ ! -f "$CONFIG_DIR/forge-web.toml" ]; then
    cp "$SCRIPT_DIR/forge-web.toml" "$CONFIG_DIR/forge-web.toml"
    sed -i '' "s|static_dir = \"./ui\"|static_dir = \"$PREFIX/share/forge/ui\"|" "$CONFIG_DIR/forge-web.toml"
    sed -i '' "s|ca_cert_path = \"./forge-data/certs/ca.crt\"|ca_cert_path = \"$DATA_DIR/certs/ca.crt\"|" "$CONFIG_DIR/forge-web.toml"
else
    echo "  Skipping forge-web.toml (already exists)"
fi

# Create data directory
echo "Creating data directory at $DATA_DIR/..."
mkdir -p "$DATA_DIR"

# ── launchd integration ───────────────────────────────────────────
LAUNCHD_DIR="/Library/LaunchDaemons"
LAUNCHD_SETUP=0
if [ -d "$LAUNCHD_DIR" ]; then
    echo ""
    echo "Installing launchd plist files..."

    cat > "$LAUNCHD_DIR/com.forge.server.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.forge.server</string>
    <key>ProgramArguments</key>
    <array>
        <string>$PREFIX/bin/forge-server</string>
        <string>serve</string>
        <string>--config</string>
        <string>$CONFIG_DIR/forge-server.toml</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>$DATA_DIR</string>
    <key>StandardOutPath</key>
    <string>/var/log/forge-server.log</string>
    <key>StandardErrorPath</key>
    <string>/var/log/forge-server.log</string>
</dict>
</plist>
EOF

    cat > "$LAUNCHD_DIR/com.forge.web.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.forge.web</string>
    <key>ProgramArguments</key>
    <array>
        <string>$PREFIX/bin/forge-web</string>
        <string>--config</string>
        <string>$CONFIG_DIR/forge-web.toml</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>$DATA_DIR</string>
    <key>StandardOutPath</key>
    <string>/var/log/forge-web.log</string>
    <key>StandardErrorPath</key>
    <string>/var/log/forge-web.log</string>
</dict>
</plist>
EOF

    LAUNCHD_SETUP=1
fi

echo ""
echo "Forge VCS Server installed successfully!"
echo ""
echo "  Binaries: $PREFIX/bin/forge-server, $PREFIX/bin/forge-web"
echo "  Config:   $CONFIG_DIR/forge-server.toml, $CONFIG_DIR/forge-web.toml"
echo "  Data:     $DATA_DIR/"
echo "  Web UI:   $PREFIX/share/forge/ui/"
echo ""

if [ "$LAUNCHD_SETUP" = "1" ]; then
    echo "Load and start both services:"
    echo "  sudo launchctl load $LAUNCHD_DIR/com.forge.server.plist"
    echo "  sudo launchctl load $LAUNCHD_DIR/com.forge.web.plist"
    echo ""
    echo "Check logs:"
    echo "  tail -f /var/log/forge-server.log"
    echo "  tail -f /var/log/forge-web.log"
else
    echo "Start manually:"
    echo "  forge-server --config $CONFIG_DIR/forge-server.toml"
    echo "  forge-web --config $CONFIG_DIR/forge-web.toml"
fi
