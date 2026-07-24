#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
componentize_log="$(mktemp)"

cleanup() {
    rm -f "${componentize_log}"
}
trap cleanup EXIT

set +e
(
    cd "${repository_root}/examples/typescript/echo-workload"
    npm run componentize
) 2>&1 | tee "${componentize_log}"
componentize_status=${PIPESTATUS[0]}
set -e

if [[ "${componentize_status}" -eq 0 ]]; then
    exit 0
fi

if grep -Fq "spidermonkey-embedding-splicer" "${componentize_log}" \
    && grep -Fq "not yet implemented" "${componentize_log}"; then
    echo "::notice::Known V2 async componentization blocker accepted; tracked by #427 and upstream ComponentizeJS#335."
    exit 0
fi

echo "::error::TypeScript workload componentization failed for an unexpected reason."
exit "${componentize_status}"
