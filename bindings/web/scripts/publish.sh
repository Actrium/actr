#!/bin/bash
set -e

# Publish all npm packages to registry

echo "Publishing packages..."

# List of packages to publish
packages=(
    "web-runtime"
    "web-sdk"
    "web-react"
)

# Build everything first
echo "Building all packages..."
npm run build

# Publish each package
for pkg in "${packages[@]}"; do
    echo "Publishing @actr/$pkg..."
    cd "packages/$pkg"

    # Check if package.json exists
    if [ ! -f "package.json" ]; then
        echo "Error: package.json not found in packages/$pkg"
        cd ../..
        continue
    fi

    # Publish with public access
    npm publish --access public || {
        echo "Warning: Failed to publish @actr/$pkg"
    }

    cd ../..
done

echo "✅ Publishing complete!"
