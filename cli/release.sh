#!/usr/bin/env bash
set -euo pipefail

# actr-cli Release Script
# Automate publishing actr-cli to crates.io

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Logging helpers
log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

# Show usage
usage() {
    cat <<EOF
Usage: $0 <version-type> --actr-version <actr-version> [options]

Version types:
  patch    Increment the patch version (0.1.0 -> 0.1.1)
  minor    Increment the minor version (0.1.0 -> 0.2.0)
  major    Increment the major version (0.1.0 -> 1.0.0)
  <version> Specify the version directly (for example 1.2.3)

Required arguments:
  --actr-version <version>  Specify the dependent actr version (it must already be published to crates.io)

Options:
  --dry-run    Validate only, do not publish
  --no-verify  Skip tests (not recommended)
  --help       Show this help message

Examples:
  $0 patch --actr-version 0.1.0           # Publish a patch release
  $0 minor --actr-version 0.2.0 --dry-run # Test a minor release
  $0 1.0.0 --actr-version 1.0.0           # Publish 1.0.0 directly

Notes:
  Before publishing, make sure the corresponding actr version has already been published to crates.io
EOF
    exit 1
}

# Parse arguments
VERSION_TYPE=""
ACTR_VERSION=""
DRY_RUN=false
NO_VERIFY=false

while [[ $# -gt 0 ]]; do
    case $1 in
        patch|minor|major)
            VERSION_TYPE="$1"
            shift
            ;;
        --actr-version)
            ACTR_VERSION="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --no-verify)
            NO_VERIFY=true
            shift
            ;;
        --help)
            usage
            ;;
        [0-9]*.[0-9]*.[0-9]*)
            VERSION_TYPE="$1"
            shift
            ;;
        *)
            log_error "Unknown argument: $1"
            usage
            ;;
    esac
done

if [[ -z "$VERSION_TYPE" ]]; then
    log_error "Please specify a version type"
    usage
fi

if [[ -z "$ACTR_VERSION" ]]; then
    log_error "Please specify the actr version (--actr-version)"
    usage
fi

# Get the current version
get_current_version() {
    grep '^version = ' Cargo.toml | head -n1 | sed -E 's/version = "(.*)"/\1/'
}

# Increment the version number
increment_version() {
    local version=$1
    local type=$2

    IFS='.' read -r major minor patch <<< "$version"

    case $type in
        major)
            echo "$((major + 1)).0.0"
            ;;
        minor)
            echo "${major}.$((minor + 1)).0"
            ;;
        patch)
            echo "${major}.${minor}.$((patch + 1))"
            ;;
        *)
            echo "$type"
            ;;
    esac
}

# Check git status
check_git_status() {
    log_info "Checking git status..."

    if [[ -n $(git status --porcelain) ]]; then
        log_error "Working tree is not clean; commit or stash your changes first"
        git status --short
        exit 1
    fi

    local branch=$(git rev-parse --abbrev-ref HEAD)
    if [[ "$branch" != "main" ]]; then
        log_warn "Current branch is not main (current: $branch)"
        read -p "Continue? [y/N] " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi

    log_success "Git status check passed"
}

# Update the version number
update_version() {
    local new_version=$1

    log_info "Updating version: $CURRENT_VERSION -> $new_version"

    sed -i.bak "s/^version = \"$CURRENT_VERSION\"/version = \"$new_version\"/" Cargo.toml
    rm -f Cargo.toml.bak

    log_success "Version updated"
}

# Back up Cargo.toml
backup_cargo_toml() {
    log_info "Backing up Cargo.toml..."
    cp Cargo.toml Cargo.toml.release-backup
}

# Restore Cargo.toml
restore_cargo_toml() {
    if [[ -f Cargo.toml.release-backup ]]; then
        log_info "Restoring Cargo.toml..."
        mv Cargo.toml.release-backup Cargo.toml
    fi
}

# Replace path dependencies with version dependencies
replace_path_with_version() {
    local actr_version=$1

    log_info "Replacing actr path dependencies with version $actr_version..."

    sed -i.tmp \
        -e "s|actr-config = { path = \"../actr/crates/config\" }|actr-config = \"$actr_version\"|g" \
        -e "s|actr-protocol = { path = \"../actr/crates/protocol\" }|actr-protocol = \"$actr_version\"|g" \
        -e "s|actr-version = { path = \"../actr/crates/version\" }|actr-version = \"$actr_version\"|g" \
        -e "s|actr-framework-protoc-codegen = { path = \"../actr/crates/framework-protoc-codegen\" }|actr-framework-protoc-codegen = \"$actr_version\"|g" \
        Cargo.toml
    rm -f Cargo.toml.tmp

    log_success "Dependency replacement completed"
}

# Run tests
run_tests() {
    if [[ "$NO_VERIFY" == true ]]; then
        log_warn "Skipping tests (--no-verify)"
        return
    fi

    log_info "Running tests..."
    cargo test --all-features
    log_success "Tests passed"
}

# Verify release settings
verify_publish() {
    log_info "Verifying release configuration..."
    cargo publish --dry-run --allow-dirty
    log_success "Release verification passed"
}

# Publish to crates.io
publish_crate() {
    if [[ "$DRY_RUN" == true ]]; then
        log_warn "Dry-run mode: skipping the actual publish"
        return
    fi

    log_info "Publishing to crates.io..."
    cargo publish
    log_success "Published to crates.io"
}

# Create the git tag
create_git_tag() {
    local version=$1
    local tag="v$version"

    if [[ "$DRY_RUN" == true ]]; then
        log_warn "Dry-run mode: skipping the git tag"
        return
    fi

    log_info "Creating git tag: $tag"

    # Commit version changes
    git add Cargo.toml Cargo.lock
    git commit -m "Release version $version"

    # Create tag
    git tag -a "$tag" -m "Release $version"

    # Push to remote
    git push origin HEAD
    git push origin "$tag"

    log_success "Git tag created and pushed: $tag"
}

# Main flow
main() {
    log_info "========================================"
    log_info "  actr-cli Release"
    log_info "========================================"
    echo

    # Get version information
    CURRENT_VERSION=$(get_current_version)
    NEW_VERSION=$(increment_version "$CURRENT_VERSION" "$VERSION_TYPE")

    log_info "Current version: $CURRENT_VERSION"
    log_info "Target version: $NEW_VERSION"
    log_info "actr dependency version: $ACTR_VERSION"

    if [[ "$DRY_RUN" == true ]]; then
        log_warn "Dry-run mode: validate only, do not publish"
    fi

    echo
    read -p "Confirm publishing actr-cli v$NEW_VERSION (depending on actr $ACTR_VERSION)? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        log_warn "Release cancelled"
        exit 0
    fi

    # Execute the release flow
    trap restore_cargo_toml ERR EXIT

    check_git_status
    backup_cargo_toml
    update_version "$NEW_VERSION"

    log_info "Updating Cargo.lock..."
    cargo update -p actr-cli

    # Dry-run mode: validate with path dependencies
    # Publish mode: replace them with version dependencies
    if [[ "$DRY_RUN" == false ]]; then
        replace_path_with_version "$ACTR_VERSION"

        log_info "Updating Cargo.lock again (using crates.io dependencies)..."
        cargo update
    fi

    run_tests
    verify_publish
    publish_crate

    # Restore Cargo.toml (while keeping the version number)
    restore_cargo_toml
    update_version "$NEW_VERSION"

    log_info "Updating the final Cargo.lock..."
    cargo update -p actr-cli

    create_git_tag "$NEW_VERSION"

    trap - ERR EXIT

    echo
    log_success "========================================"
    log_success "  actr-cli $NEW_VERSION release completed!"
    log_success "========================================"
    echo

    if [[ "$DRY_RUN" == false ]]; then
        log_info "Crates.io: https://crates.io/crates/actr-cli"
        log_info "The new version should be available in a few minutes"
    fi
}

main
