#!/bin/bash
# Shared helper for start scripts: ensure actr.toml and actrix-config.toml exist by copying from example files if needed.

# Function to ensure actr.toml exists by copying from Actr.example.toml if needed
ensure_actr_toml() {
    local dir=$1
    local actr_toml="$dir/actr.toml"
    local actr_example="$dir/Actr.example.toml"
    
    # Use color variables if defined, otherwise use empty strings
    local red="${RED:-}"
    local nc="${NC:-}"
    
    if [ ! -f "$actr_toml" ]; then
        if [ -f "$actr_example" ]; then
            echo "📋 Copying $actr_example to $actr_toml"
            cp "$actr_example" "$actr_toml"
        else
            echo -e "${red}❌ Actr.example.toml not found at $actr_example${nc}" >&2
            return 1
        fi
    fi
}

ensure_actrix_config() {
    local workspace_root=$1
    local config_toml="${ACTRIX_CONFIG:-$workspace_root/actrix-config.toml}"
    local config_example="$workspace_root/actrix-config.example.toml"
    
    # Try to find actrix project directory relative to workspace root (assuming it's in Actrium/actrix)
    local actrix_dir_local="${ACTRIX_DIR:-$workspace_root/../../../../actrix}"
    local config_example_actrix="$actrix_dir_local/config.example.toml"
    
    # Use color variables if defined, otherwise use empty strings
    local red="${RED:-}"
    local nc="${NC:-}"
    
    if [ ! -f "$config_toml" ]; then
        if [ -f "$config_example" ]; then
            echo "📋 Copying $config_example to $config_toml"
            cp "$config_example" "$config_toml"
        elif [ -f "$config_example_actrix" ]; then
            echo "📋 Copying $config_example_actrix to $config_toml"
            cp "$config_example_actrix" "$config_toml"
        else
            echo -e "${red}❌ actrix config example not found at $config_example or $config_example_actrix${nc}" >&2
            return 1
        fi
    fi
}

