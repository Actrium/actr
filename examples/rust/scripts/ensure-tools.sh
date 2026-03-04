#!/bin/bash
# Shared helper for start scripts: ensure required CLI tools exist.

# Get installed version of a cargo-installed binary
get_installed_version() {
    local bin="$1"
    if ! command -v "$bin" >/dev/null 2>&1; then
        return 1
    fi
    
    # Try to get version from --version or -V flag
    local version_output
    version_output=$("$bin" --version 2>/dev/null || "$bin" -V 2>/dev/null || echo "")
    if [ -z "$version_output" ]; then
        return 1
    fi
    
    # Extract version number (handles formats like "tool 1.2.3" or "tool v1.2.3")
    echo "$version_output" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1
}

# Get latest version from crates.io (with timeout to avoid hanging)
get_latest_version() {
    local crate="$1"
    local latest_version
    local timeout_cmd=""
    local timeout_sec=10
    
    # Try to find timeout command (GNU coreutils on macOS via Homebrew, or built-in on Linux)
    if command -v gtimeout >/dev/null 2>&1; then
        timeout_cmd="gtimeout"
    elif command -v timeout >/dev/null 2>&1; then
        timeout_cmd="timeout"
    fi
    
    # Use timeout if available, otherwise use a background process approach
    if [ -n "$timeout_cmd" ]; then
        latest_version=$($timeout_cmd $timeout_sec cargo search --registry crates-io --limit 1 "$crate" 2>/dev/null | grep "^$crate" | awk '{print $3}' | tr -d '"')
    else
        # Fallback: use a temporary file and background process with timeout
        local tmp_file=$(mktemp)
        (cargo search --registry crates-io --limit 1 "$crate" 2>/dev/null | grep "^$crate" | awk '{print $3}' | tr -d '"' > "$tmp_file") &
        local pid=$!
        
        # Wait up to timeout_sec seconds
        local count=0
        while kill -0 $pid 2>/dev/null && [ $count -lt $timeout_sec ]; do
            sleep 1
            count=$((count + 1))
        done
        
        # Kill if still running
        if kill -0 $pid 2>/dev/null; then
            kill $pid 2>/dev/null || true
            wait $pid 2>/dev/null || true
            rm -f "$tmp_file"
            return 1
        fi
        
        wait $pid 2>/dev/null || true
        if [ -f "$tmp_file" ]; then
            latest_version=$(cat "$tmp_file")
            rm -f "$tmp_file"
        fi
    fi
    
    if [ -z "$latest_version" ]; then
        return 1
    fi
    echo "$latest_version"
}

# Compare version strings (returns 0 if v1 >= v2, 1 otherwise)
version_ge() {
    local v1="$1"
    local v2="$2"
    
    # Use sort -V for version comparison
    if [ "$(printf '%s\n' "$v1" "$v2" | sort -V | head -1)" = "$v2" ]; then
        return 0  # v1 >= v2
    else
        return 1  # v1 < v2
    fi
}

# ensure_cargo_bin <binary> <crate> <log_dir>
ensure_cargo_bin() {
    local bin="$1"
    local crate="$2"
    local log_dir="$3"
    local install_log="$log_dir/cargo-install-${bin}.log"

    mkdir -p "$log_dir"

    # protoc-gen-prost version constraint: requires 0.5.x for flat_output_dir support
    # See: https://github.com/neoeinstein/protoc-gen-prost/blob/main/CHANGELOG.md#050---2025-11-19
    if [ "$bin" = "protoc-gen-prost" ]; then
        local required_version="0.5.0"
        local max_tested_version="0.5"  # Major.Minor prefix for compatibility check

        if command -v "$bin" >/dev/null 2>&1; then
            local installed_version
            installed_version=$("$bin" --version 2>/dev/null || echo "")

            if [ -n "$installed_version" ]; then
                # Check if version is less than required
                if ! version_ge "$installed_version" "$required_version"; then
                    echo "[warn] $bin version $installed_version is below required $required_version"
                    echo "[info] Installing $bin@$required_version for flat_output_dir support..."
                    if ! cargo install "$crate" --version "$required_version" --force > "$install_log" 2>&1; then
                        echo "[error] Failed to install $bin (crate: $crate). Check $install_log for details."
                        return 1
                    fi
                    echo "[ok] Installed $bin $required_version: $(command -v "$bin")"
                    return 0
                fi

                # Check if version is higher than tested (warn but continue)
                local installed_major_minor
                installed_major_minor=$(echo "$installed_version" | grep -oE '^[0-9]+\.[0-9]+')
                if [ "$installed_major_minor" != "$max_tested_version" ] && ! version_ge "$max_tested_version.999" "$installed_version"; then
                    echo "[warn] $bin version $installed_version is newer than tested version $max_tested_version.x"
                    echo "[warn] There may be compatibility risks. See: https://github.com/neoeinstein/protoc-gen-prost/blob/main/CHANGELOG.md"
                fi

                echo "[ok] $bin already installed at compatible version ($installed_version): $(command -v "$bin")"
                return 0
            fi

            # Version detection failed, assume it's ok
            echo "[ok] $bin already installed (version unknown): $(command -v "$bin")"
            return 0
        fi

        # Not installed, install required version
        echo "[info] Installing $bin@$required_version via cargo install $crate (log: $install_log)..."
        if ! cargo install "$crate" --version "$required_version" > "$install_log" 2>&1; then
            echo "[error] Failed to install $bin (crate: $crate). Check $install_log for details."
            return 1
        fi
        if ! command -v "$bin" >/dev/null 2>&1; then
            echo "[error] $bin still unavailable after install. Ensure \$HOME/.cargo/bin is in PATH."
            return 1
        fi
        echo "[ok] Installed $bin $required_version: $(command -v "$bin")"
        return 0
    fi

    # Check if binary already exists
    if command -v "$bin" >/dev/null 2>&1; then
        local installed_version
        local latest_version
        
        installed_version=$(get_installed_version "$bin" 2>/dev/null || echo "")
        latest_version=$(get_latest_version "$crate" 2>/dev/null || echo "")
        
        if [ -n "$installed_version" ] && [ -n "$latest_version" ]; then
            if version_ge "$installed_version" "$latest_version"; then
                echo "[ok] $bin already installed at latest version ($installed_version): $(command -v "$bin")"
                return 0
            else
                echo "[info] $bin installed version ($installed_version) is older than latest ($latest_version), updating..."
            fi
        elif [ -n "$installed_version" ]; then
            echo "[info] $bin already installed (version $installed_version), skipping version check (network issue?), updating anyway..."
        else
            echo "[info] $bin already installed, checking for updates..."
        fi
        
        echo "[info] Updating $bin via cargo install --force $crate (log: $install_log)..."
        if ! cargo install --force "$crate" > "$install_log" 2>&1; then
            echo "[error] Failed to update $bin (crate: $crate). Check $install_log for details."
            return 1
        fi
        echo "[ok] Updated $bin: $(command -v "$bin")"
        return 0
    fi

    echo "[info] Installing $bin via cargo install $crate (log: $install_log)..."
    mkdir -p "$log_dir"
    if ! cargo install "$crate" > "$install_log" 2>&1; then
        echo "[error] Failed to install $bin (crate: $crate). Check $install_log for details."
        return 1
    fi

    if ! command -v "$bin" >/dev/null 2>&1; then
        echo "[error] $bin still unavailable after install. Ensure \$HOME/.cargo/bin is in PATH."
        return 1
    fi

    echo "[ok] Installed $bin: $(command -v "$bin")"
}
