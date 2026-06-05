#!/usr/bin/env bash
set -euo pipefail

# Package libactr.so native libraries for Android distribution:
# - Zips jniLibs into dist/actr-kotlin-native.zip
# - Computes SHA256 checksum
# - Prints the URL/checksum pair for Release asset upload
#
# Usage:
#   ./scripts/package-binary.sh v0.2.0
#     - Uses the provided tag for the Release download URL.
#   ACTR_BINARY_TAG=v0.2.0 ./scripts/package-binary.sh
#     - Or set via environment variable.
#
# Prerequisites:
# - Run ./build-android.sh first to generate jniLibs/{arm64-v8a,x86_64}/libactr.so
# - sha256sum (or shasum -a 256 on macOS)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
resolve_root_path() {
  local path="$1"
  if [[ "${path}" = /* ]]; then
    printf '%s\n' "${path}"
  else
    printf '%s\n' "${ROOT_DIR}/${path}"
  fi
}

DIST_DIR="$(resolve_root_path "${ACTR_DIST_DIR:-dist}")"
JNILIBS_DIR="$(resolve_root_path "${ACTR_JNILIBS_DIR:-actr-kotlin/src/main/jniLibs}")"
ZIP_PATH="${DIST_DIR}/actr-kotlin-native.zip"
RELEASE_REPOSITORY="${ACTR_RELEASE_REPOSITORY:-Actrium/actr-kotlin-package-sync}"

RELEASE_TAG="${1:-${ACTR_BINARY_TAG:-v0.1.0}}"

# Detect checksum command (sha256sum on Linux, shasum on macOS)
if command -v sha256sum >/dev/null 2>&1; then
  CHECKSUM_CMD="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  CHECKSUM_CMD="shasum -a 256"
else
  echo "error: neither sha256sum nor shasum found" >&2
  exit 1
fi

# Validate input
if [[ ! -d "${JNILIBS_DIR}/arm64-v8a" ]] || [[ ! -f "${JNILIBS_DIR}/arm64-v8a/libactr.so" ]]; then
  echo "error: missing ${JNILIBS_DIR}/arm64-v8a/libactr.so; run ./build-android.sh first" >&2
  exit 1
fi

if [[ ! -d "${JNILIBS_DIR}/x86_64" ]] || [[ ! -f "${JNILIBS_DIR}/x86_64/libactr.so" ]]; then
  echo "error: missing ${JNILIBS_DIR}/x86_64/libactr.so; run ./build-android.sh first" >&2
  exit 1
fi

mkdir -p "${DIST_DIR}"
rm -f "${ZIP_PATH}" "${DIST_DIR}/release.txt"

echo "[1/3] Zipping native libraries -> ${ZIP_PATH}"
ZIP_SOURCE_PARENT="$(dirname "${JNILIBS_DIR}")"
ZIP_SOURCE_NAME="$(basename "${JNILIBS_DIR}")"
(cd "${ZIP_SOURCE_PARENT}" && zip -qry "${ZIP_PATH}" "${ZIP_SOURCE_NAME}")

echo "[2/3] Computing SHA256 checksum"
CHECKSUM="$(${CHECKSUM_CMD} "${ZIP_PATH}" | awk '{print $1}')"

DOWNLOAD_URL="https://github.com/${RELEASE_REPOSITORY}/releases/download/${RELEASE_TAG}/actr-kotlin-native.zip"

echo "[3/3] Release info"
cat > "${DIST_DIR}/release.txt" <<EOF
Release tag:     ${RELEASE_TAG}
Download URL:    ${DOWNLOAD_URL}
SHA256 checksum: ${CHECKSUM}

Archive contents:
  jniLibs/
    arm64-v8a/libactr.so   (aarch64-linux-android)
    x86_64/libactr.so      (x86_64-linux-android)

Upload asset to GitHub Release:
  gh release upload ${RELEASE_TAG} ${ZIP_PATH} --clobber
EOF

echo ""
echo "✅ Packaged ${ZIP_PATH}"
echo "🔑 Checksum: ${CHECKSUM}"
echo ""
echo "Next steps:"
echo "  1) Upload ${ZIP_PATH} to GitHub Release: ${RELEASE_TAG}"
echo "  2) Repository: https://github.com/${RELEASE_REPOSITORY}"
echo "  3) Download URL: ${DOWNLOAD_URL}"
echo ""
