#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: bash scripts/publish.sh [--dry-run] [--expected-version <version>] [--skip-build] [--tag <npm-dist-tag>]

Publishes the Actrium web npm packages in dependency order:
  1. @actrium/actr-dom
  2. @actrium/actr-web
  3. @actrium/actr-web-react

The script uses pnpm to create workspace-aware tarballs, then uses npm publish
for the final publish step so GitHub Actions Trusted Publishing can use OIDC.
EOF
}

dry_run=0
skip_build=0
expected_version=""
npm_tag="latest"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      dry_run=1
      shift
      ;;
    --skip-build)
      skip_build=1
      shift
      ;;
    --tag)
      npm_tag="${2:-}"
      if [ -z "$npm_tag" ]; then
        echo "--tag requires a value" >&2
        exit 2
      fi
      shift 2
      ;;
    --expected-version)
      expected_version="${2:-}"
      if [ -z "$expected_version" ]; then
        echo "--expected-version requires a value" >&2
        exit 2
      fi
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

PACKAGES=(
  "packages/actr-dom:@actrium/actr-dom"
  "packages/web-sdk:@actrium/actr-web"
  "packages/web-react:@actrium/actr-web-react"
)

cd "$WEB_ROOT"

for package in "${PACKAGES[@]}"; do
  path="${package%%:*}"
  expected_name="${package##*:}"
  actual_name="$(node -p "require('./${path}/package.json').name")"
  actual_version="$(node -p "require('./${path}/package.json').version")"

  if [ "$actual_name" != "$expected_name" ]; then
    echo "Expected $path package name $expected_name, got $actual_name" >&2
    exit 1
  fi

  if [ -n "$expected_version" ] && [ "$actual_version" != "$expected_version" ]; then
    echo "Expected $expected_name version $expected_version, got $actual_version" >&2
    exit 1
  fi
done

if [ "$skip_build" -eq 0 ]; then
  pnpm run build:packages
fi

pack_dir="$(mktemp -d)"
trap 'rm -rf "$pack_dir"' EXIT

publish_args=(publish --access public)
if [ "$npm_tag" != "latest" ]; then
  publish_args+=(--tag "$npm_tag")
fi
if [ "$dry_run" -eq 1 ]; then
  publish_args+=(--dry-run)
fi

for package in "${PACKAGES[@]}"; do
  path="${package%%:*}"
  expected_name="${package##*:}"
  echo "Packing $expected_name from $path"
  pack_output="$(cd "$path" && pnpm pack --pack-destination "$pack_dir" --json)"
  tarball="$(node -e '
    const fs = require("node:fs");
    const input = fs.readFileSync(0, "utf8");
    const data = JSON.parse(input);
    const entry = Array.isArray(data) ? data[0] : data;
    if (!entry?.filename) {
      throw new Error(`pnpm pack did not return a tarball filename: ${input}`);
    }
    process.stdout.write(entry.filename);
  ' <<<"$pack_output")"

  if [ ! -f "$tarball" ]; then
    echo "Packed tarball does not exist: $tarball" >&2
    exit 1
  fi

  echo "Publishing $expected_name from $tarball"
  npm "${publish_args[@]}" "$tarball"
done
