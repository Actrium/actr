#!/bin/bash
# Shared helper for start scripts: ensure Actr.toml and actrix-config.toml exist by copying from example files if needed.

# Function to ensure Actr.toml exists by copying from Actr.example.toml if needed
ensure_actr_toml() {
    local dir=$1
    local actr_toml="$dir/Actr.toml"
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

# Function to ensure actrix-config.toml exists by copying from actrix-config.example.toml if needed
ensure_actrix_config() {
    local workspace_root=$1
    local config_toml="$workspace_root/actrix-config.toml"
    local config_example="$workspace_root/actrix-config.example.toml"
    
    # Use color variables if defined, otherwise use empty strings
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

