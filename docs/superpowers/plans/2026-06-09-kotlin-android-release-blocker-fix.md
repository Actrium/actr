# Kotlin Android Release Blocker Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Permanently fix the Kotlin Android release blocker in the Actr source repository, add regression coverage, and remove release-time dependency on package-sync hotpatching.

**Architecture:** Fix the source Android build script so opus linker flags are stored per Android target and the relink command uses valid Bash subshell syntax. Add a cheap static regression test and wire it into Kotlin CI so script syntax and cross-ABI RUSTFLAGS mixing are caught before release. After the source fix is merged and released, simplify the Kotlin package-sync workflow from mutating cloned source to validating cloned source.

**Tech Stack:** Bash, GitHub Actions, Gradle/Kotlin CI, Android NDK toolchain, jj colocated repository workflow, GitHub CLI.

---

## Current Evidence

- `Actrium/actr` release run `27196401733` completed successfully on attempt 3 for `v0.3.4`, but that only proves the source release finished.
- `Actrium/actr-kotlin-package-sync` run `27200324473` completed successfully and published `v0.3.4`, but it used a workflow step named `Patch Android build script if needed`.
- `origin/main:bindings/kotlin/build-android.sh` still fails `bash -n` with a syntax error at the relink subshell:

```text
line 267: syntax error near unexpected token `('
```

- The same source script still accumulates opus linker flags globally:

```bash
RUSTFLAGS_EXTRA="${RUSTFLAGS_EXTRA} -L ${opus_lib_dir} -l opus"
```

That global accumulation caused the second blocker: the x86_64 relink saw the aarch64 opus path and `ld.lld` rejected the incompatible `libopus.so`.

## Files

- Modify: `bindings/kotlin/build-android.sh`
  - Store opus linker flags per Android ABI instead of in one global `RUSTFLAGS_EXTRA`.
  - Use valid Bash syntax for `RUSTFLAGS=... cargo build` inside the relink subshell.
- Create: `scripts/tests/kotlin-android-build-script.sh`
  - Run `bash -n` on `bindings/kotlin/build-android.sh`.
  - Assert the script no longer contains global opus RUSTFLAGS accumulation.
  - Assert the relink loop derives `target_upper` and uses the matching target-specific RUSTFLAGS.
- Modify: `.github/workflows/ci-kotlin.yml`
  - Add a fast script validation job before Gradle-heavy Kotlin jobs.
- Follow-up in sibling repo: `/Users/kaito/Project/Actrium/actr-kotlin-package-sync/.github/workflows/release.yml`
  - After an Actr source release includes this fix, remove the future-facing hotpatch and keep validation only.

## Task 1: Fix Source Android Build Script

**Files:**
- Modify: `bindings/kotlin/build-android.sh`

- [ ] **Step 1: Inspect the working tree before editing**

Run:

```bash
jj st
git status --short --branch
```

Expected: existing unrelated changes, if any, are identified and left untouched. If implementing outside the current dirty workspace, create an isolated workspace using the worktree skill before editing.

- [ ] **Step 2: Open one coherent jj change if the implementation workspace is clean**

Run:

```bash
jj new -m "wip: fix kotlin android release blocker"
```

Expected: one new change is created for this fix. Skip this step only if the implementation workspace is already inside the intended change.

- [ ] **Step 3: Replace global RUSTFLAGS accumulation in `fix_opus_for_target`**

Find:

```bash
    # Expose the lib dir for RUSTFLAGS
    RUSTFLAGS_EXTRA="${RUSTFLAGS_EXTRA} -L ${opus_lib_dir} -l opus"
```

Replace with:

```bash
    # Expose the target-specific lib dir for RUSTFLAGS.
    printf -v "RUSTFLAGS_EXTRA_${target_upper}" "%s" "-L ${opus_lib_dir} -l opus"
```

Why: `target_upper` is `aarch64` or `x86_64`, both valid Bash variable suffixes. This keeps each ABI's opus library directory isolated and avoids linking x86_64 against an aarch64 `libopus.so`.

- [ ] **Step 4: Remove the obsolete global initializer**

Find:

```bash
RUSTFLAGS_EXTRA=""
```

Replace with:

```bash
unset RUSTFLAGS_EXTRA_aarch64 RUSTFLAGS_EXTRA_x86_64
```

Why: the relink phase should fail if the matching target-specific flags were not produced. It should not silently fall back to an empty or mixed global value.

- [ ] **Step 5: Fix the relink loop target metadata and subshell syntax**

Find the relink loop:

```bash
for target_pair in "aarch64-linux-android aarch64" "x86_64-linux-android x86_64"; do
    target=$(echo "$target_pair" | awk '{print $1}')
    # Force relink
    rm -f "${TARGET_DIR}/${target}/release/libactr.so"
    find "${TARGET_DIR}/${target}/release/deps" -name "liblibactr*" -delete 2>/dev/null
    find "${TARGET_DIR}/${target}/release/.fingerprint" -name "libactr-*" -maxdepth 1 -exec rm -rf {} + 2>/dev/null

    RUSTFLAGS="${RUSTFLAGS_EXTRA}" \
        (cd "${WORKSPACE_ROOT}" && cargo build -p libactr --release --target "${target}")
done
```

Replace it with:

```bash
for target_pair in "aarch64-linux-android aarch64" "x86_64-linux-android x86_64"; do
    target=$(echo "$target_pair" | awk '{print $1}')
    target_upper=$(echo "$target_pair" | awk '{print $2}')
    target_rustflags_var="RUSTFLAGS_EXTRA_${target_upper}"
    target_rustflags="${!target_rustflags_var:?missing opus RUSTFLAGS for ${target}}"

    # Force relink
    rm -f "${TARGET_DIR}/${target}/release/libactr.so"
    find "${TARGET_DIR}/${target}/release/deps" -name "liblibactr*" -delete 2>/dev/null
    find "${TARGET_DIR}/${target}/release/.fingerprint" -name "libactr-*" -maxdepth 1 -exec rm -rf {} + 2>/dev/null

    (
        cd "${WORKSPACE_ROOT}"
        RUSTFLAGS="${target_rustflags}" cargo build -p libactr --release --target "${target}"
    )
done
```

Expected:
- `bash -n bindings/kotlin/build-android.sh` passes.
- The relink phase uses only the current target's generated `-L ... -l opus` value.
- Missing target flags fail early with a clear message.

## Task 2: Add Regression Script

**Files:**
- Create: `scripts/tests/kotlin-android-build-script.sh`

- [ ] **Step 1: Create the shell regression test**

Create `scripts/tests/kotlin-android-build-script.sh` with:

```bash
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

if 'RUSTFLAGS_EXTRA="${RUSTFLAGS_EXTRA} -L ${opus_lib_dir} -l opus"' in text:
    fail("build-android.sh must not accumulate opus RUSTFLAGS globally")

if 'printf -v "RUSTFLAGS_EXTRA_${target_upper}" "%s" "-L ${opus_lib_dir} -l opus"' not in text:
    fail("build-android.sh must store opus RUSTFLAGS by target")

marker = '==> Relinking libactr with libopus.so DT_NEEDED...'
marker_index = text.find(marker)
if marker_index == -1:
    fail("build-android.sh relink marker not found")

relink = text[marker_index:]

required = [
    'target_upper=$(echo "$target_pair" | awk \'{print $2}\')',
    'target_rustflags_var="RUSTFLAGS_EXTRA_${target_upper}"',
    'target_rustflags="${!target_rustflags_var:?missing opus RUSTFLAGS for ${target}}"',
    'RUSTFLAGS="${target_rustflags}" cargo build -p libactr --release --target "${target}"',
]

for snippet in required:
    if snippet not in relink:
        fail(f"build-android.sh relink loop missing expected snippet: {snippet}")

invalid = 'RUSTFLAGS="${RUSTFLAGS_EXTRA}" \\\n        (cd "${WORKSPACE_ROOT}" && cargo build -p libactr --release --target "${target}")'
if invalid in text:
    fail("build-android.sh still contains invalid env-assignment subshell syntax")
PY
```

- [ ] **Step 2: Mark the script executable**

Run:

```bash
chmod +x scripts/tests/kotlin-android-build-script.sh
```

Expected:

```text
scripts/tests/kotlin-android-build-script.sh
```

is executable.

- [ ] **Step 3: Run the regression script**

Run:

```bash
scripts/tests/kotlin-android-build-script.sh
```

Expected: exit code `0` and no stderr.

## Task 3: Wire Regression Into Kotlin CI

**Files:**
- Modify: `.github/workflows/ci-kotlin.yml`

- [ ] **Step 1: Add a fast script validation job**

Insert this job before the existing `test` job:

```yaml
  script-check:
    name: Kotlin Android Script Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - name: Validate Android build script
        run: scripts/tests/kotlin-android-build-script.sh
```

Expected: `.github/workflows/ci-kotlin.yml` has three jobs: `script-check`, `test`, and `gen-test`.

- [ ] **Step 2: Validate workflow YAML shape**

Run:

```bash
python3 - <<'PY'
from pathlib import Path
import sys

path = Path(".github/workflows/ci-kotlin.yml")
text = path.read_text()
for snippet in [
    "script-check:",
    "Kotlin Android Script Check",
    "scripts/tests/kotlin-android-build-script.sh",
]:
    if snippet not in text:
        print(f"missing {snippet}", file=sys.stderr)
        raise SystemExit(1)
PY
```

Expected: exit code `0`.

## Task 4: Local Verification

**Files:**
- Verify: `bindings/kotlin/build-android.sh`
- Verify: `scripts/tests/kotlin-android-build-script.sh`
- Verify: `.github/workflows/ci-kotlin.yml`

- [ ] **Step 1: Run Bash syntax validation directly**

Run:

```bash
bash -n bindings/kotlin/build-android.sh
```

Expected: exit code `0`.

- [ ] **Step 2: Run the regression script**

Run:

```bash
scripts/tests/kotlin-android-build-script.sh
```

Expected: exit code `0`.

- [ ] **Step 3: Run formatting check required by repository rules**

Run:

```bash
cargo fmt --all --check
```

Expected: exit code `0`. If formatting is already clean, no files change.

- [ ] **Step 4: Run Kotlin CI-equivalent Gradle checks if local Java/Gradle setup is available**

Run:

```bash
(cd bindings/kotlin && ./gradlew ktlintCheck :actr-kotlin:test :actr-kotlin:build --no-daemon)
```

Expected: exit code `0`. If this fails because local toolchains are missing, do not claim Kotlin build validation locally; rely on remote CI for this step.

- [ ] **Step 5: Run Android build only when Android SDK and NDK are available**

Run:

```bash
ANDROID_HOME="${ANDROID_HOME:?set ANDROID_HOME}" \
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME}" \
bash bindings/kotlin/build-android.sh
```

Expected:
- Both `aarch64-linux-android` and `x86_64-linux-android` builds complete.
- Relink does not emit an incompatible `libopus.so` error.

- [ ] **Step 6: Inspect DT_NEEDED entries after a successful Android build**

Run:

```bash
find bindings/kotlin -path '*jniLibs*' -name libactr.so -print
find bindings/kotlin -path '*jniLibs*' -name libopus.so -print
```

If `llvm-readelf` from the Android NDK is available, run:

```bash
case "$(uname -s)" in
  Darwin) host_tag="darwin-x86_64" ;;
  Linux) host_tag="linux-x86_64" ;;
  *) echo "unsupported host for Android NDK prebuilt lookup: $(uname -s)" >&2; exit 1 ;;
esac

"${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/${host_tag}/bin/llvm-readelf" -d \
  bindings/kotlin/actr-kotlin/src/main/jniLibs/arm64-v8a/libactr.so | grep 'NEEDED.*libopus.so'
"${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/${host_tag}/bin/llvm-readelf" -d \
  bindings/kotlin/actr-kotlin/src/main/jniLibs/x86_64/libactr.so | grep 'NEEDED.*libopus.so'
```

Expected: each ABI's `libactr.so` needs `libopus.so` and does not fail ABI compatibility checks during relink.

## Task 5: Publish Source PR and Verify Remote CI

**Files:**
- Publish: `bindings/kotlin/build-android.sh`
- Publish: `scripts/tests/kotlin-android-build-script.sh`
- Publish: `.github/workflows/ci-kotlin.yml`

- [ ] **Step 1: Finalize jj description**

Run:

```bash
jj describe -m "fix(kotlin): harden Android release build script"
```

Expected: the change title follows Conventional Commits and describes one functional intent.

- [ ] **Step 2: Create or move a dedicated bookmark**

Run:

```bash
jj bookmark set harry/fix-kotlin-android-release-build -r @
```

Expected: the bookmark points at the finalized change, not at `main`.

- [ ] **Step 3: Push the bookmark**

Run:

```bash
jj git push --bookmark harry/fix-kotlin-android-release-build
```

Expected: remote branch is pushed successfully.

- [ ] **Step 4: Open a PR to `main`**

Run:

```bash
gh pr create \
  --base main \
  --head harry/fix-kotlin-android-release-build \
  --title "fix(kotlin): harden Android release build script" \
  --body-file /tmp/actr-kotlin-android-release-build-pr.md
```

Use this PR body:

```markdown
## Summary
- fix Android build script relink syntax
- isolate opus linker flags per Android target
- add a fast Kotlin Android build script regression check to CI

## Validation
- `bash -n bindings/kotlin/build-android.sh`
- `scripts/tests/kotlin-android-build-script.sh`
- `cargo fmt --all --check`
```

Expected: a PR URL is returned.

- [ ] **Step 5: Watch remote checks**

Run:

```bash
gh pr checks --watch
```

If any check fails, inspect logs with:

```bash
failed_run_id=$(
  gh run list \
    --repo Actrium/actr \
    --branch harry/fix-kotlin-android-release-build \
    --limit 10 \
    --json databaseId,conclusion,status \
    --jq '.[] | select(.conclusion == "failure" or .conclusion == "cancelled") | .databaseId' \
    | head -n1
)

gh run view "${failed_run_id}" --repo Actrium/actr --json jobs,url,conclusion,status

failed_job_id=$(
  gh run view "${failed_run_id}" \
    --repo Actrium/actr \
    --json jobs \
    --jq '.jobs[] | select(.conclusion == "failure") | .databaseId' \
    | head -n1
)

gh run view "${failed_run_id}" --repo Actrium/actr --job "${failed_job_id}" --log
```

Expected:
- `CI (Kotlin) / Kotlin Android Script Check` succeeds.
- Existing Kotlin test and codegen jobs succeed or failures are diagnosed from logs before proceeding.

- [ ] **Step 6: Merge only after CI is green**

Run:

```bash
gh pr merge --squash --delete-branch
```

Expected: source fix lands on `main`.

## Task 6: Remove Future-Facing Package-Sync Hotpatch

**Files:**
- Modify in sibling repo: `/Users/kaito/Project/Actrium/actr-kotlin-package-sync/.github/workflows/release.yml`

- [ ] **Step 1: Wait until the Actr source fix is on `main`**

Run:

```bash
source_pr_number=$(
  gh pr view harry/fix-kotlin-android-release-build \
    --repo Actrium/actr \
    --json number \
    --jq '.number'
)

gh pr view "${source_pr_number}" --repo Actrium/actr --json state,mergedAt,mergeCommit
```

Expected: `state` is `MERGED` and `mergeCommit` is not null.

- [ ] **Step 2: Replace source mutation with source validation**

In `/Users/kaito/Project/Actrium/actr-kotlin-package-sync/.github/workflows/release.yml`, replace the `Patch Android build script if needed` Python hotpatch step with:

```yaml
      - name: Validate Android build script
        run: bash -n source-checkout/bindings/kotlin/build-android.sh
```

Why: future package-sync releases should fail if source is broken instead of silently modifying source during release.

- [ ] **Step 3: Open a package-sync PR**

Run from `/Users/kaito/Project/Actrium/actr-kotlin-package-sync`:

```bash
git checkout -B harry/remove-kotlin-source-hotpatch origin/main
git add .github/workflows/release.yml
git commit -m "fix(release): validate Android build script without hotpatching"
git push -u origin harry/remove-kotlin-source-hotpatch
gh pr create \
  --repo Actrium/actr-kotlin-package-sync \
  --base main \
  --head harry/remove-kotlin-source-hotpatch \
  --title "fix(release): validate Android build script without hotpatching" \
  --body "## Summary
- remove release-time mutation of the cloned Actr Android build script
- keep Bash syntax validation before Android/Kotlin release build

## Validation
- source Actr PR is merged
- package-sync workflow syntax is valid"
```

Expected: a package-sync PR URL is returned.

- [ ] **Step 4: Merge package-sync PR after checks are green**

Run:

```bash
gh pr checks --repo Actrium/actr-kotlin-package-sync --watch
gh pr merge --repo Actrium/actr-kotlin-package-sync --squash --delete-branch
```

Expected: the package-sync workflow no longer hotpatches current/future Actr source releases.

## Task 7: End-to-End Release Verification

**Files:**
- Verify source repo: `Actrium/actr`
- Verify downstream repo: `Actrium/actr-kotlin-package-sync`

- [ ] **Step 1: Trigger or wait for the next source release that includes the source fix**

Run:

```bash
gh run list --repo Actrium/actr --workflow release-train.yml --limit 10
```

Expected: identify the run for the fixed version, for example `v0.3.5` or the next prepared version.

- [ ] **Step 2: Confirm source release completed**

Run:

```bash
source_release_run_id=$(
  gh run list \
    --repo Actrium/actr \
    --workflow release-train.yml \
    --limit 20 \
    --json databaseId,status,conclusion \
    --jq '.[] | select(.status == "completed" and .conclusion == "success") | .databaseId' \
    | head -n1
)

gh run view "${source_release_run_id}" \
  --repo Actrium/actr \
  --json status,conclusion,url,attempt,headSha,updatedAt
```

Expected: `status` is `completed`, `conclusion` is `success`, and `headSha` contains the source fix.

- [ ] **Step 3: Confirm Kotlin package-sync run completed without source hotpatching**

Run:

```bash
gh run list --repo Actrium/actr-kotlin-package-sync --workflow release.yml --limit 10

kotlin_package_sync_run_id=$(
  gh run list \
    --repo Actrium/actr-kotlin-package-sync \
    --workflow release.yml \
    --limit 20 \
    --json databaseId,status,conclusion \
    --jq '.[] | select(.status == "completed" and .conclusion == "success") | .databaseId' \
    | head -n1
)

gh run view "${kotlin_package_sync_run_id}" \
  --repo Actrium/actr-kotlin-package-sync \
  --json status,conclusion,url,jobs
```

Expected:
- `conclusion` is `success`.
- Job steps include `Validate Android build script`.
- Job steps do not include `Patch Android build script if needed`.

- [ ] **Step 4: Confirm downstream release exists**

Run:

```bash
fixed_version=$(
  gh release list \
    --repo Actrium/actr \
    --limit 1 \
    --json tagName \
    --jq '.[0].tagName'
)

gh release view "${fixed_version}" \
  --repo Actrium/actr-kotlin-package-sync \
  --json tagName,url,publishedAt,isDraft,isPrerelease
```

Expected:
- `isDraft` is `false`.
- `isPrerelease` matches the source release.
- `url` points to the Kotlin package-sync release for the fixed version.

## Definition of Done

This blocker is permanently fixed only when all of these are true:

- `origin/main:bindings/kotlin/build-android.sh` passes `bash -n`.
- The script no longer accumulates opus RUSTFLAGS globally.
- The relink loop uses target-specific RUSTFLAGS for `aarch64-linux-android` and `x86_64-linux-android`.
- Kotlin CI contains and passes `Kotlin Android Script Check`.
- The next Actr source release containing the fix succeeds.
- The Kotlin package-sync run for that fixed source release succeeds without a source hotpatch step.
- The Kotlin package-sync GitHub Release for the fixed version exists and is not a draft.

Until the package-sync hotpatch is removed or bypassed for current/future source releases, the state should be described as “release rescued by downstream hotpatch,” not “source blocker permanently fixed.”
