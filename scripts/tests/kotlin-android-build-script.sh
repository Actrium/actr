#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
script_path="${repo_root}/bindings/kotlin/build-android.sh"

bash -n "${script_path}"

python3 - "${script_path}" <<'PY'
from pathlib import Path
import sys

script_path = Path(sys.argv[1])
text = script_path.read_text()


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


required = [
    'ACTR_ANDROID_TARGETS="${ACTR_ANDROID_TARGETS:-aarch64-linux-android x86_64-linux-android}"',
    'ACTR_BUILD_ANDROID_NATIVE="${ACTR_BUILD_ANDROID_NATIVE:-true}"',
    'ACTR_BUILD_HOST_LIBRARY="${ACTR_BUILD_HOST_LIBRARY:-true}"',
    'ACTR_GENERATE_KOTLIN_BINDINGS="${ACTR_GENERATE_KOTLIN_BINDINGS:-true}"',
    'target_upper_for()',
    'target_abi_for()',
    'copy_target_if_dir_exists()',
    'printf -v "RUSTFLAGS_EXTRA_${target_upper}" "%s" "-L ${opus_lib_dir} -l opus"',
    'target_rustflags="${!target_rustflags_var:?missing opus RUSTFLAGS for ${target}}"',
    'RUSTFLAGS="${target_rustflags}" cargo build -p libactr --release --target "${target}"',
]

for snippet in required:
    if snippet not in text:
        fail(f"build-android.sh missing expected snippet: {snippet}")

for forbidden in [
    'RUSTFLAGS_EXTRA="${RUSTFLAGS_EXTRA} -L ${opus_lib_dir} -l opus"',
    'RUSTFLAGS="${RUSTFLAGS_EXTRA}" \\\n        (cd "${WORKSPACE_ROOT}" && cargo build -p libactr --release --target "${target}")',
]:
    if forbidden in text:
        fail(f"build-android.sh still contains forbidden snippet: {forbidden}")
PY
