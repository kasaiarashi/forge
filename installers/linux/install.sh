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

    # Older release tarballs (pre-.gitattributes) were packaged on the
    # Windows CI runner with core.autocrlf=true, so the in-tarball
    # install.sh shipped with CRLF line endings. bash then reads
    # `set -euo pipefail\r` and rejects `pipefail\r` as an invalid
    # option. Strip CRs defensively so the bootstrap stays forward-
    # compatible with already-published releases (it's a no-op on
    # tarballs that were already LF).
    tr -d '\r' < "$extracted/install.sh" > "$extracted/install.sh.lf"
    mv "$extracted/install.sh.lf" "$extracted/install.sh"
    chmod +x "$extracted/install.sh"

    # Hand off to the in-tarball copy of this script. `exec` replaces
    # the current process so the trap above also drops out — but the
    # tmpdir is what we need to keep around until install.sh is done,
    # so we let install.sh inherit the trap by running it as a child
    # rather than exec'ing.
    PREFIX="$PREFIX" CONFIG_DIR="$CONFIG_DIR" DATA_DIR="$DATA_DIR" \
        bash "$extracted/install.sh"
    exit $?
fi

# ── Interactive location prompts ───────────────────────────────────────
#
# Env vars (PREFIX/CONFIG_DIR/DATA_DIR) still win when set explicitly. We
# read from /dev/tty rather than stdin so prompts work even under
# `curl ... | sudo bash`, where stdin is the piped script and a
# naive `[ -t 0 ]` check would skip the prompts entirely. Skip only when
# /dev/tty isn't available (truly non-interactive: CI, docker build, etc.).
# FORGE_NONINTERACTIVE=1 forces the non-prompting path for scripted installs.
if [ -z "${FORGE_NONINTERACTIVE:-}" ] && [ -r /dev/tty ] && [ -w /dev/tty ]; then
    {
        echo "Forge VCS Server installer"
        echo ""
        echo "Press Enter to accept the default path shown in brackets, or type a"
        echo "different absolute path to install somewhere else."
        echo ""
    } > /dev/tty

    printf "Install prefix (binaries + web UI) [%s]: " "$PREFIX" > /dev/tty
    IFS= read -r _answer < /dev/tty || _answer=""
    [ -n "$_answer" ] && PREFIX="$_answer"

    printf "Config dir (TOML files)              [%s]: " "$CONFIG_DIR" > /dev/tty
    IFS= read -r _answer < /dev/tty || _answer=""
    [ -n "$_answer" ] && CONFIG_DIR="$_answer"

    printf "Data dir (objects, DB, certs)        [%s]: " "$DATA_DIR" > /dev/tty
    IFS= read -r _answer < /dev/tty || _answer=""
    [ -n "$_answer" ] && DATA_DIR="$_answer"
    echo "" > /dev/tty
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
    # The shipped default ca_cert_path is ./forge-data/certs/ca.crt,
    # which assumes base_path = ./forge-data. In the installed layout
    # base_path IS the data dir, so certs end up at $DATA_DIR/certs/,
    # not $DATA_DIR/forge-data/certs/. Rewrite to the absolute path
    # so forge-web can actually verify forge-server's auto-generated
    # self-signed CA.
    sed -i "s|ca_cert_path = \"./forge-data/certs/ca.crt\"|ca_cert_path = \"$DATA_DIR/certs/ca.crt\"|" "$CONFIG_DIR/forge-web.toml"
else
    echo "  Skipping forge-web.toml (already exists)"
fi

# Create data directory
echo "Creating data directory at $DATA_DIR/..."
install -d "$DATA_DIR"

# ── systemd integration ────────────────────────────────────────────
#
# Only set up systemd units when systemd is actually PID 1. Checking
# `command -v systemctl` alone isn't enough: WSL1 and some containers
# ship the binary without a running systemd. `/run/systemd/system`
# only exists when systemd is the active init, so it's the canonical
# "is systemd live" check. Skipping gracefully keeps the installer
# working on WSL1, Docker, chroots, etc.
SYSTEMD_SETUP=0
if [ -d /run/systemd/system ]; then
    echo ""
    echo "Detected systemd — installing unit files..."

    # Dedicated unprivileged system user for the daemons. No login
    # shell, no home dir — purely an identity for dropping privileges
    # in the units below. Idempotent so re-running the installer is
    # safe.
    if ! getent passwd forge >/dev/null; then
        echo "  Creating 'forge' system user..."
        useradd --system \
                --no-create-home \
                --home-dir "$DATA_DIR" \
                --shell /usr/sbin/nologin \
                forge
    fi

    # forge-server writes objects, refs, and auto-generated TLS
    # material under $DATA_DIR, so the forge user needs to own it.
    # Group-writable + setgid so admins in the 'forge' group can run
    # CLI management commands (e.g. forge-server user add) directly.
    chown -R forge:forge "$DATA_DIR"
    chmod 2775 "$DATA_DIR"

    # Generate unit files with the actual install paths baked in.
    # Shipping static units would hardcode /usr/local and break when
    # PREFIX is overridden. Unquoted heredoc = variable expansion.
    cat > /etc/systemd/system/forge-server.service <<EOF
[Unit]
Description=Forge VCS gRPC server
Documentation=https://github.com/kasaiarashi/forge
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$PREFIX/bin/forge-server serve --config $CONFIG_DIR/forge-server.toml
Restart=on-failure
RestartSec=5
User=forge
Group=forge
WorkingDirectory=$DATA_DIR

# Security hardening — cheap wins that don't affect functionality.
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=$DATA_DIR

[Install]
WantedBy=multi-user.target
EOF

    cat > /etc/systemd/system/forge-web.service <<EOF
[Unit]
Description=Forge VCS web UI
Documentation=https://github.com/kasaiarashi/forge
After=network-online.target forge-server.service
Wants=network-online.target
# forge-server is Wants (not Requires) so this unit can also be used
# to drive a remote forge-server instance by editing
# $CONFIG_DIR/forge-web.toml's grpc_url + ca_cert_path.

[Service]
Type=simple
ExecStart=$PREFIX/bin/forge-web --config $CONFIG_DIR/forge-web.toml
Restart=on-failure
RestartSec=5
User=forge
Group=forge
WorkingDirectory=$DATA_DIR

AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=$DATA_DIR

[Install]
WantedBy=multi-user.target
EOF

    chmod 644 /etc/systemd/system/forge-server.service
    chmod 644 /etc/systemd/system/forge-web.service

    # Add the invoking user to the forge group so they can run CLI admin
    # commands (forge-server user add, etc.) without sudo. Try $SUDO_USER
    # first (installer run via sudo — most common case), fall back to
    # `logname` (controlling tty's login name) when the script was run
    # directly as root. Skip for root itself since root already has access.
    INVOKING_USER="${SUDO_USER:-}"
    if [ -z "$INVOKING_USER" ] && command -v logname >/dev/null 2>&1; then
        INVOKING_USER="$(logname 2>/dev/null || true)"
    fi
    if [ -n "$INVOKING_USER" ] && [ "$INVOKING_USER" != "root" ] \
       && id "$INVOKING_USER" >/dev/null 2>&1 \
       && ! id -nG "$INVOKING_USER" 2>/dev/null | grep -qw forge; then
        echo "  Adding '$INVOKING_USER' to 'forge' group..."
        usermod -aG forge "$INVOKING_USER"
        GROUP_ADDED_USER="$INVOKING_USER"
    fi

    systemctl daemon-reload

    # Enable + start both services so the install is immediately usable.
    # Prior installs left users to run `systemctl enable --now` manually,
    # which was easy to miss — the services just never came up and
    # forge-web's TLS bind never initialized certs. Idempotent: re-running
    # the installer on an already-running system is a no-op.
    echo "Enabling and starting forge-server and forge-web..."
    systemctl enable forge-server.service forge-web.service
    systemctl restart forge-server.service forge-web.service

    SYSTEMD_SETUP=1
fi

# ── Clean up the old profile.d wrapper if present ──────────────────────
# Earlier installer versions shipped /etc/profile.d/forge.sh as a shell
# function that injected --config. It only worked for bash login shells;
# zsh/fish users never picked it up. The forge-server binary now falls
# back to /etc/forge/forge-server.toml internally when the user doesn't
# pass --config, so the wrapper is obsolete. Remove it on upgrade to
# avoid two layers of indirection.
if [ -f /etc/profile.d/forge.sh ]; then
    rm -f /etc/profile.d/forge.sh
fi

echo ""
echo "Forge VCS Server installed successfully!"
echo ""
echo "  Binaries: $PREFIX/bin/forge-server, $PREFIX/bin/forge-web"
echo "  Config:   $CONFIG_DIR/forge-server.toml, $CONFIG_DIR/forge-web.toml"
echo "  Data:     $DATA_DIR/"
echo "  Web UI:   $PREFIX/share/forge/ui/"
echo ""

if [ "$SYSTEMD_SETUP" = "1" ]; then
    echo "Enable and start both services:"
    echo "  systemctl enable --now forge-server forge-web"
    echo ""
    echo "Check status / tail logs:"
    echo "  systemctl status forge-server forge-web"
    echo "  journalctl -u forge-server -f"
    echo "  journalctl -u forge-web -f"
    echo ""
    echo "Create your first admin user:"
    echo "  sudo forge-server user add <username> --admin"
    echo ""
    echo "(run via sudo so the 'forge'-owned data dir is writable; alternatively"
    echo "log out + back in after group membership changes and drop the sudo)"
    if [ -n "${GROUP_ADDED_USER:-}" ]; then
        echo ""
        echo "Note: '$GROUP_ADDED_USER' was added to the 'forge' group and the"
        echo "data dir is group-writable. Log out + back in (or run"
        echo "'newgrp forge') so the new group membership takes effect."
    fi
else
    echo "systemd not detected — start manually:"
    echo "  forge-server --config $CONFIG_DIR/forge-server.toml"
    echo "  forge-web --config $CONFIG_DIR/forge-web.toml"
fi
