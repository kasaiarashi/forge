#!/usr/bin/env bash
set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
CONFIG_DIR="${CONFIG_DIR:-/etc/forge}"
DATA_DIR="${DATA_DIR:-/var/lib/forge}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

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
