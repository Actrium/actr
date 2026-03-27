#!/bin/bash
# Shared helper for start scripts: ensure manifest.toml, actr.toml and actrix-config.toml
# exist by copying from example files if needed.
#
# New two-file layout:
#   manifest.toml  <- Actr.example.toml  (package manifest, signed)
#   actr.toml      <- Hyper.example.toml (runtime config, env-specific)

# Ensure manifest.toml exists (copy from Actr.example.toml)
ensure_manifest_toml() {
    local dir=$1
    local manifest_toml="$dir/manifest.toml"
    local manifest_example="$dir/Actr.example.toml"

    local red="${RED:-}"
    local nc="${NC:-}"

    if [ ! -f "$manifest_toml" ]; then
        if [ -f "$manifest_example" ]; then
            echo "📋 Copying $manifest_example -> $manifest_toml"
            cp "$manifest_example" "$manifest_toml"
        else
            echo -e "${red}❌ Actr.example.toml not found at $manifest_example${nc}" >&2
            return 1
        fi
    fi
}

# Ensure actr.toml (runtime config) exists (copy from Hyper.example.toml)
ensure_actr_toml() {
    local dir=$1
    local actr_toml="$dir/actr.toml"
    local hyper_example="$dir/Hyper.example.toml"

    local red="${RED:-}"
    local nc="${NC:-}"

    if [ ! -f "$actr_toml" ]; then
        if [ -f "$hyper_example" ]; then
            echo "📋 Copying $hyper_example -> $actr_toml"
            cp "$hyper_example" "$actr_toml"
        else
            echo -e "${red}❌ Hyper.example.toml not found at $hyper_example${nc}" >&2
            return 1
        fi
    fi
}

# Ensure actrix-config.toml exists by copying from actrix-config.example.toml if needed
ensure_actrix_config() {
    local workspace_root=$1
    local config_toml="$workspace_root/actrix-config.toml"
    local config_example="$workspace_root/actrix-config.example.toml"

    local red="${RED:-}"
    local nc="${NC:-}"

    if [ ! -f "$config_toml" ]; then
        if [ -f "$config_example" ]; then
            echo "📋 Copying $config_example to $config_toml"
            cp "$config_example" "$config_toml"
        else
            echo -e "${red}❌ actrix-config.example.toml not found at $config_example${nc}" >&2
            return 1
        fi
    fi
}
