#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_root"

# Load release-train helpers without executing the script entrypoint.
script_under_test=$(mktemp)
sed '/^main "\$@"$/d' scripts/release-train.sh >"$script_under_test"
source "$script_under_test"
trap 'rm -f "$script_under_test"' EXIT

PACKAGE_SYNC_GITHUB_TOKEN="test-token"
PACKAGE_SYNC_OWNER="Actrium"

curl() {
  cat <<'JSON'
{
  "workflow_runs": [
    {
      "id": 26830046367,
      "created_at": "2026-06-02T15:27:49Z"
    }
  ]
}
JSON
}

run_id=$(find_package_sync_run_id \
  "actr-swift-package-sync" \
  "release.yml" \
  "2026-06-02T15:19:29Z")

if [[ "$run_id" != "26830046367" ]]; then
  printf 'expected run id 26830046367, got %s\n' "${run_id:-<empty>}" >&2
  exit 1
fi
