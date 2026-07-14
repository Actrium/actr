#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
script_path="${repo_root}/bindings/kotlin/build-android.sh"
publication_path="${repo_root}/bindings/kotlin/actr-kotlin/build.gradle.kts"
harness_path="${repo_root}/e2e/kotlin-lib/lib/kotlin-app.sh"
fixture_path="${repo_root}/cli/fixtures/kotlin/app/build.gradle.kts"

bash -n "${script_path}"

python3 - "${script_path}" "${publication_path}" "${harness_path}" <<'PY'
from pathlib import Path
import sys

script_path = Path(sys.argv[1])
publication_path = Path(sys.argv[2])
harness_path = Path(sys.argv[3])
text = script_path.read_text()
publication_text = publication_path.read_text()
harness_text = harness_path.read_text()


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

if 'artifactId = "actr"' not in publication_text:
    fail("Kotlin Maven publication must explicitly use artifactId `actr`")

if 'artifactId = "actr-kotlin"' in publication_text:
    fail("Kotlin Maven publication must not use the legacy `actr-kotlin` artifactId")

if '-PactrVersion="$ACTR_KOTLIN_VERSION"' not in harness_text:
    fail("local Kotlin publication must use ACTR_KOTLIN_VERSION")
PY

tmpdir=$(mktemp -d)
trap 'rm -rf "${tmpdir}"' EXIT
app_dir="${tmpdir}/app"
mkdir -p "${app_dir}/app"
cp "${fixture_path}" "${app_dir}/app/build.gradle.kts"

ACTR_KOTLIN_VERSION="0.0.0-test"
REPO_ROOT="${repo_root}"
source "${harness_path}"

section() { :; }
warn() { :; }
success() { :; }
resolve_android_sdk_root() { return 1; }
render_template() {
    local template="$1"
    local output="$2"
    sed 's/__PROJECT_NAME__/coordinate-test/g' "${template}" >"${output}"
}

substitute_local_aar "${app_dir}"
grep -Fq 'implementation("io.actrium:actr:0.0.0-test")' "${app_dir}/app/build.gradle.kts"
if grep -Fq 'io.actrium:actr-kotlin' "${app_dir}/app/build.gradle.kts"; then
    echo "local Kotlin AAR wiring used the legacy artifactId" >&2
    exit 1
fi
