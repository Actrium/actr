#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# download-actrix-artifact.sh
#
# Shared script used by both Linux Package Runtime E2E and macOS Swift E2E
# jobs to download a pre-built actrix binary from the latest successful
# Actrium/actrix CI run instead of cloning and building from source.
#
# Usage:
#   bash download-actrix-artifact.sh <actrix-linux-x86_64|actrix-macos-arm64>
#
# Required environment variables:
#   ACTR_E2E_ACTRIX_ARTIFACT_REPO      e.g. Actrium/actrix
#   ACTR_E2E_ACTRIX_ARTIFACT_WORKFLOW  workflow ID (string)
#   ACTR_E2E_ACTRIX_ARTIFACT_BRANCH    e.g. main
#   GH_TOKEN                           GitHub token with actions:read scope
#   GITHUB_ENV                         Path to GitHub Actions env file
#   RUNNER_TEMP                        GitHub Actions temp directory
#
# Outputs:
#   Writes "ACTRIX_BIN=<absolute-path>" to GITHUB_ENV so that subsequent
#   steps and shell scripts (e.g. ensure_actrix_available) can use it
#   directly.
# ──────────────────────────────────────────────────────────────────────────

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

section() {
    echo ""
    echo -e "${BLUE}━━━ $1 ━━━${NC}"
}

success() {
    echo -e "${GREEN}✅ $1${NC}"
}

fail() {
    echo -e "${RED}❌ $1${NC}" >&2
    exit 1
}

# ── Parse artifact name ─────────────────────────────────────────────────

ARTIFACT_NAME="${1:-}"
if [ -z "$ARTIFACT_NAME" ]; then
    fail "Usage: $0 <actrix-linux-x86_64|actrix-macos-arm64>"
fi

case "$ARTIFACT_NAME" in
    actrix-linux-x86_64|actrix-macos-arm64) ;;
    *)
        fail "Invalid artifact name: $ARTIFACT_NAME (expected actrix-linux-x86_64 or actrix-macos-arm64)"
        ;;
esac

# ── Validate prerequisites ──────────────────────────────────────────────

section "Validating prerequisites"

for cmd in gh jq unzip; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        fail "Required command not found: $cmd"
    fi
done
success "All required commands available"

: "${ACTR_E2E_ACTRIX_ARTIFACT_REPO:?ACTR_E2E_ACTRIX_ARTIFACT_REPO is required}"
: "${ACTR_E2E_ACTRIX_ARTIFACT_WORKFLOW:?ACTR_E2E_ACTRIX_ARTIFACT_WORKFLOW is required}"
: "${ACTR_E2E_ACTRIX_ARTIFACT_BRANCH:?ACTR_E2E_ACTRIX_ARTIFACT_BRANCH is required}"
: "${GH_TOKEN:?GH_TOKEN is required (set to ACTRIX_READ_TOKEN secret)}"
: "${GITHUB_ENV:?GITHUB_ENV is required (must run in GitHub Actions)}"
: "${RUNNER_TEMP:?RUNNER_TEMP is required (must run in GitHub Actions)}"

success "All required environment variables set"

# ── Resolve latest successful workflow run ─────────────────────────────

section "Resolving latest successful actrix workflow run"

RUNS_JSON=$(gh api \
    "repos/${ACTR_E2E_ACTRIX_ARTIFACT_REPO}/actions/workflows/${ACTR_E2E_ACTRIX_ARTIFACT_WORKFLOW}/runs?branch=${ACTR_E2E_ACTRIX_ARTIFACT_BRANCH}&status=success&per_page=1")

if ! echo "$RUNS_JSON" | jq -e '.workflow_runs' >/dev/null 2>&1; then
    echo "gh api returned unexpected response (check token scope, repo, or workflow ID):" >&2
    echo "$RUNS_JSON" | jq '.' >&2 || true
    fail "Failed to query workflow runs from ${ACTR_E2E_ACTRIX_ARTIFACT_REPO}"
fi

RUN_ID=$(echo "$RUNS_JSON" | jq -r '.workflow_runs[0].id // ""')
HEAD_SHA=$(echo "$RUNS_JSON" | jq -r '.workflow_runs[0].head_sha // ""')

if [ -z "$RUN_ID" ] || [ "$RUN_ID" = "null" ]; then
    echo "No successful runs found for workflow. Verify:" >&2
    echo "  - Repo:   ${ACTR_E2E_ACTRIX_ARTIFACT_REPO}" >&2
    echo "  - Branch: ${ACTR_E2E_ACTRIX_ARTIFACT_BRANCH}" >&2
    echo "  - Token scope includes actions:read" >&2
    fail "No successful actrix CI run found on branch ${ACTR_E2E_ACTRIX_ARTIFACT_BRANCH}"
fi

echo "  Repository: $ACTR_E2E_ACTRIX_ARTIFACT_REPO"
echo "  Workflow:   $ACTR_E2E_ACTRIX_ARTIFACT_WORKFLOW"
echo "  Branch:     $ACTR_E2E_ACTRIX_ARTIFACT_BRANCH"
echo "  Run ID:     $RUN_ID"
echo "  Head SHA:   $HEAD_SHA"

success "Selected actrix run id=$RUN_ID sha=$HEAD_SHA"

# ── Verify artifact exists and is not expired ───────────────────────────

section "Verifying artifact availability"

ARTIFACTS_JSON=$(gh api \
    "repos/${ACTR_E2E_ACTRIX_ARTIFACT_REPO}/actions/runs/${RUN_ID}/artifacts")

if ! echo "$ARTIFACTS_JSON" | jq -e '.artifacts' >/dev/null 2>&1; then
    echo "gh api returned unexpected response (check token scope or run ID):" >&2
    echo "$ARTIFACTS_JSON" | jq '.' >&2 || true
    fail "Failed to list artifacts for run $RUN_ID"
fi

ARTIFACT_EXPIRED=$(echo "$ARTIFACTS_JSON" | jq -er \
    --arg name "$ARTIFACT_NAME" \
    '.artifacts[] | select(.name == $name) | .expired' 2>/dev/null || echo "")

if [ -z "$ARTIFACT_EXPIRED" ]; then
    echo "Available artifacts:" >&2
    echo "$ARTIFACTS_JSON" | jq -r '.artifacts[] | "  - \(.name) (expired: \(.expired))"' >&2
    fail "Artifact '$ARTIFACT_NAME' not found in run $RUN_ID (may be expired)"
fi

if [ "$ARTIFACT_EXPIRED" = "true" ]; then
    fail "Artifact '$ARTIFACT_NAME' exists in run $RUN_ID but has expired"
fi

success "Artifact '$ARTIFACT_NAME' is available (not expired)"

# ── Prepare download directory ──────────────────────────────────────────

ACTRIX_DIR="${RUNNER_TEMP}/actrix"
rm -rf "$ACTRIX_DIR"
mkdir -p "$ACTRIX_DIR"

# ── Download artifact ──────────────────────────────────────────────────

section "Downloading artifact"

gh run download "$RUN_ID" \
    -R "$ACTR_E2E_ACTRIX_ARTIFACT_REPO" \
    -n "$ARTIFACT_NAME" \
    -D "$ACTRIX_DIR"

# ── Locate actrix binary ───────────────────────────────────────────────

ACTRIX_BIN=$(find "$ACTRIX_DIR" -name actrix -type f | head -1)
if [ -z "$ACTRIX_BIN" ]; then
    echo "Contents of $ACTRIX_DIR:" >&2
    find "$ACTRIX_DIR" -type f -o -type l >&2
    fail "actrix binary not found in downloaded artifact"
fi

chmod +x "$ACTRIX_BIN"
success "actrix binary located at $ACTRIX_BIN"

# ── Verify binary architecture and executability ────────────────────────

section "Verifying actrix binary"

"$ACTRIX_BIN" --version || fail "actrix --version failed (binary may be corrupted or wrong architecture)"
success "actrix binary verified"

# ── Export to GITHUB_ENV ───────────────────────────────────────────────

echo "ACTRIX_BIN=$ACTRIX_BIN" >> "$GITHUB_ENV"
success "ACTRIX_BIN=$ACTRIX_BIN written to GITHUB_ENV"
