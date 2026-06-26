#!/bin/bash
# Actrix Installation Script (LEGACY)
#
# ⚠️  DEPRECATED — kept for reference only. Prefer the `deploy` tool
#     (`cargo install --path .` → `deploy service`/`deploy update`), which
#     implements the releases/symlink model, SHA-256 verification, atomic
#     binary swap, and systemd hardening. This legacy script does NONE of
#     that: it uses a flat /opt/actrix/actrix layout with no checksum, no
#     version pinning, and overwrites the running binary in place. Do not use
#     it for production deployments.
#
# This script helps install Actrix as a systemd service
#
# Usage:
#   sudo ./install.sh [install|uninstall|update]

set -e

echo -e "${YELLOW}⚠️  install.sh is LEGACY and lacks checksum verification / releases model.${NC}"
echo -e "${YELLOW}    Use the 'deploy' tool instead: deploy service / deploy update.${NC}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
INSTALL_DIR="/opt/actrix"
SERVICE_NAME="actrix"
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
USER="actrix"
GROUP="actrix"

# Print colored message
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Generate a systemd service unit file from script configuration.
generate_service_file() {
    local output_file="$1"
    cat > "$output_file" <<EOF
[Unit]
Description=Actrix Auxiliary Services
Documentation=https://github.com/Actrium/actrix
After=network.target

[Service]
Type=simple
User=${USER}
Group=${GROUP}

# Environment variables
Environment=RUST_LOG=info
Environment=RUST_BACKTRACE=1
Environment=ACTRIX_PATH=${INSTALL_DIR}

# Working directory
WorkingDirectory=${INSTALL_DIR}

# Start command
ExecStart=${INSTALL_DIR}/actrix --config ${INSTALL_DIR}/config.toml

# Reload configuration without restart
ExecReload=/bin/kill -HUP \$MAINPID

# Restart policy
Restart=always
RestartSec=5

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=actrix

# Security settings
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${INSTALL_DIR}/logs ${INSTALL_DIR}/database.db

# Resource limits
LimitNOFILE=65536
LimitNPROC=4096
LimitMEMLOCK=infinity

[Install]
WantedBy=multi-user.target
EOF
}

# Check if script is run as root
check_root() {
    if [ "$EUID" -ne 0 ]; then
        print_error "This script must be run as root"
        exit 1
    fi
}

# Check if binary exists
check_binary() {
    if [ ! -f "target/release/actrix" ]; then
        print_error "Binary not found. Please build the project first:"
        print_error "  cargo build --release"
        exit 1
    fi
}

# Create user and group
create_user() {
    if id "$USER" &>/dev/null; then
        print_info "User $USER already exists"
    else
        print_info "Creating user $USER..."
        useradd --system --no-create-home --shell /bin/false "$USER"
    fi
}

# Install actrix
install_actrix() {
    print_info "Installing Actrix to $INSTALL_DIR..."

    # Create installation directory
    mkdir -p "$INSTALL_DIR"
    mkdir -p "$INSTALL_DIR/logs"
    mkdir -p "$INSTALL_DIR/certificates"

    # Copy binary
    print_info "Copying binary..."
    cp target/release/actrix "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/actrix"

    # Copy configuration
    if [ -f "config.toml" ]; then
        print_info "Copying existing config.toml..."
        cp config.toml "$INSTALL_DIR/"
    elif [ -f "config.example.toml" ]; then
        print_info "Copying config.example.toml as config.toml..."
        cp config.example.toml "$INSTALL_DIR/config.toml"
        print_warn "Please edit $INSTALL_DIR/config.toml before starting the service"
    else
        print_error "No configuration file found"
        exit 1
    fi

    # Set ownership
    print_info "Setting ownership..."
    chown -R "$USER:$GROUP" "$INSTALL_DIR"
    chmod 600 "$INSTALL_DIR/config.toml"

    # Install systemd service
    print_info "Installing systemd service..."
    local service_tmp
    service_tmp="$(mktemp)"
    generate_service_file "$service_tmp"
    cp "$service_tmp" "$SERVICE_FILE"
    chmod 644 "$SERVICE_FILE"
    rm -f "$service_tmp"
    systemctl daemon-reload

    print_info "Installation completed!"
    print_info ""
    print_info "Next steps:"
    print_info "  1. Edit configuration: nano $INSTALL_DIR/config.toml"
    print_info "  2. Add certificates to: $INSTALL_DIR/certificates/"
    print_info "  3. Enable service: systemctl enable $SERVICE_NAME"
    print_info "  4. Start service: systemctl start $SERVICE_NAME"
    print_info "  5. Check status: systemctl status $SERVICE_NAME"
    print_info "  6. View logs: journalctl -u $SERVICE_NAME -f"
}

# Uninstall actrix
uninstall_actrix() {
    print_info "Uninstalling Actrix..."

    # Stop and disable service
    if systemctl is-active --quiet "$SERVICE_NAME"; then
        print_info "Stopping service..."
        systemctl stop "$SERVICE_NAME"
    fi

    if systemctl is-enabled --quiet "$SERVICE_NAME"; then
        print_info "Disabling service..."
        systemctl disable "$SERVICE_NAME"
    fi

    # Remove service file
    if [ -f "$SERVICE_FILE" ]; then
        print_info "Removing service file..."
        rm "$SERVICE_FILE"
        systemctl daemon-reload
    fi

    # Ask before removing installation directory
    read -p "Remove installation directory $INSTALL_DIR? [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        print_info "Removing $INSTALL_DIR..."
        rm -rf "$INSTALL_DIR"
    else
        print_warn "Installation directory preserved at $INSTALL_DIR"
    fi

    # Ask before removing user
    read -p "Remove user $USER? [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        print_info "Removing user $USER..."
        userdel "$USER" 2>/dev/null || true
    fi

    print_info "Uninstallation completed!"
}

# Update actrix binary
update_actrix() {
    print_info "Updating Actrix..."

    # Check if service is running
    WAS_RUNNING=false
    if systemctl is-active --quiet "$SERVICE_NAME"; then
        WAS_RUNNING=true
        print_info "Stopping service..."
        systemctl stop "$SERVICE_NAME"
    fi

    # Backup old binary
    if [ -f "$INSTALL_DIR/actrix" ]; then
        print_info "Backing up old binary..."
        cp "$INSTALL_DIR/actrix" "$INSTALL_DIR/actrix.backup"
    fi

    # Copy new binary
    print_info "Copying new binary..."
    cp target/release/actrix "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/actrix"
    chown "$USER:$GROUP" "$INSTALL_DIR/actrix"

    # Update service file if changed
    local service_tmp
    service_tmp="$(mktemp)"
    generate_service_file "$service_tmp"
    if ! cmp -s "$service_tmp" "$SERVICE_FILE"; then
        print_info "Updating service file..."
        cp "$service_tmp" "$SERVICE_FILE"
        chmod 644 "$SERVICE_FILE"
        systemctl daemon-reload
    fi
    rm -f "$service_tmp"

    # Restart service if it was running
    if [ "$WAS_RUNNING" = true ]; then
        print_info "Restarting service..."
        systemctl start "$SERVICE_NAME"
    fi

    print_info "Update completed!"
    print_info "Binary version:"
    "$INSTALL_DIR/actrix" --version || print_warn "Could not determine version"
}

# Show usage
show_usage() {
    echo "Usage: $0 [install|uninstall|update]"
    echo ""
    echo "Commands:"
    echo "  install     Install Actrix as a systemd service"
    echo "  uninstall   Remove Actrix and systemd service"
    echo "  update      Update Actrix binary (preserves config)"
    echo ""
    echo "Examples:"
    echo "  sudo ./install.sh install"
    echo "  sudo ./install.sh update"
    echo "  sudo ./install.sh uninstall"
}

# Main
main() {
    check_root

    case "${1:-}" in
        install)
            check_binary
            create_user
            install_actrix
            ;;
        uninstall)
            uninstall_actrix
            ;;
        update)
            check_binary
            update_actrix
            ;;
        *)
            show_usage
            exit 1
            ;;
    esac
}

main "$@"
