#!/usr/bin/env python3

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CI_GATE_WORKFLOW = ROOT / ".github/workflows/ci-gate.yml"
CI_E2E_WORKFLOW = ROOT / ".github/workflows/ci-e2e.yml"
RELEASE_TRAIN_WORKFLOW = ROOT / ".github/workflows/release-train.yml"
RELEASE_TRAIN_SCRIPT = ROOT / "scripts/release-train.sh"


def _job(workflow: str, name: str, next_name: str) -> str:
    job_start = workflow.index(f"  {name}:\n")
    next_job_start = workflow.index(f"\n  {next_name}:\n", job_start)
    return workflow[job_start:next_job_start]


def test_rust_gate_avoids_slow_workspace_tests_and_unused_prewarm() -> None:
    workflow = CI_GATE_WORKFLOW.read_text(encoding="utf-8")
    rust_job = _job(workflow, "rust", "test")

    assert "- name: Run tests" not in rust_job
    assert "cargo test --workspace" not in rust_job
    assert "- name: Prepare Rust codegen plugins" not in rust_job
    assert "cargo install protoc-gen-prost" not in rust_job
    assert "cargo build --quiet -p actr-framework-protoc-codegen" not in rust_job
    assert "- name: Build workspace" not in rust_job
    assert "cargo build --verbose --all-features" not in rust_job
    assert "- name: Build release" not in rust_job
    assert "cargo build --release --verbose --all-features" not in rust_job

    # The separate test job (not inside rust) runs cargo test on push to main
    assert "  test:" in workflow
    test_job = _job(workflow, "test", "typescript")
    assert "- name: Run tests" in test_job
    assert "cargo test --workspace" in test_job


def test_pr_gate_excludes_heavy_root_e2e_jobs() -> None:
    workflow = CI_GATE_WORKFLOW.read_text(encoding="utf-8")

    assert "  ts_stream_e2e:\n" not in workflow
    assert "  web_browser_e2e:\n" not in workflow
    assert "ts_stream_e2e" not in workflow
    assert "web_browser_e2e" not in workflow


def test_scheduled_e2e_runs_root_level_browser_and_stream_e2e() -> None:
    workflow = CI_E2E_WORKFLOW.read_text(encoding="utf-8")

    assert "e2e/typescript-stream/**" in workflow
    assert "e2e/web-browser/**" in workflow
    assert "bash e2e/typescript-stream/run.sh" in workflow
    assert "bash e2e/web-browser/run.sh" in workflow


def test_pr_gate_swift_uses_macos_only_xcframework() -> None:
    workflow = CI_GATE_WORKFLOW.read_text(encoding="utf-8")
    swift_job = _job(workflow, "swift", "kotlin")

    assert "ACTR_XCFRAMEWORK_TARGETS: macos" in swift_job
    assert "targets: aarch64-apple-darwin" in swift_job
    assert "targets: aarch64-apple-ios,aarch64-apple-ios-sim,aarch64-apple-darwin" not in swift_job


def test_release_train_has_valid_publish_steps() -> None:
    raw_workflow = RELEASE_TRAIN_WORKFLOW.read_bytes()
    assert all(byte >= 0x20 or byte in b"\n\r\t" for byte in raw_workflow)

    workflow = raw_workflow.decode("utf-8")
    for stage in (
        "publish-rust",
        "publish-python",
        "publish-swift",
        "publish-kotlin",
        "publish-web",
        "publish-typescript-workload",
        "publish-typescript",
    ):
        assert f"- name: Run {stage} stage" in workflow


def test_release_train_waits_for_matching_ci_gate() -> None:
    workflow = RELEASE_TRAIN_WORKFLOW.read_text(encoding="utf-8")
    gate_job = _job(workflow, "gate", "context")

    assert "actions: read" in workflow
    assert "- name: Wait for CI Gate" in gate_job
    assert "actions/workflows/ci-gate.yml/runs" in gate_job
    assert "head_sha=${RELEASE_SHA}" in gate_job


def test_release_train_forwards_release_context() -> None:
    workflow = RELEASE_TRAIN_WORKFLOW.read_text(encoding="utf-8")
    release_script = RELEASE_TRAIN_SCRIPT.read_text(encoding="utf-8")
    create_tag_job = _job(workflow, "create-tag", "publish-rust")

    assert "EXPECTED_RELEASE_SHA" in create_tag_job
    assert 'needs.context.outputs.release_sha }}' in create_tag_job

    stage_jobs = (
        ("create-tag", "publish-rust"),
        ("publish-rust", "publish-python"),
        ("publish-python", "publish-swift"),
        ("publish-swift", "publish-kotlin"),
        ("publish-kotlin", "publish-web"),
        ("publish-web", "build-typescript-native"),
        ("publish-typescript-workload", "publish-typescript"),
        ("publish-typescript", "collect-report"),
    )
    for job, next_job in stage_jobs:
        job_workflow = _job(workflow, job, next_job)
        assert 'needs.context.outputs.skip_python }}" == "true"' in job_workflow
        assert "args+=(--skip-python)" in job_workflow
        assert 'needs.context.outputs.pre_release }}" == "true"' in job_workflow
        assert "args+=(--pre-release)" in job_workflow

    report_job = workflow[workflow.index("  collect-report:\n") :]
    assert 'needs.context.outputs.skip_python }}" == "true"' in report_job
    assert "args+=(--skip-python)" in report_job
    assert 'needs.context.outputs.pre_release }}" == "true"' in report_job
    assert "args+=(--pre-release)" in report_job

    assert '[[ "$STAGE" == "report" ]]' in release_script
    assert 'current_sha=$(git rev-parse HEAD)' in release_script
    assert 'Release context SHA ${RELEASE_SHA} does not match current HEAD ${current_sha}' in release_script


if __name__ == "__main__":
    test_rust_gate_avoids_slow_workspace_tests_and_unused_prewarm()
    test_pr_gate_excludes_heavy_root_e2e_jobs()
    test_scheduled_e2e_runs_root_level_browser_and_stream_e2e()
    test_pr_gate_swift_uses_macos_only_xcframework()
    test_release_train_has_valid_publish_steps()
    test_release_train_waits_for_matching_ci_gate()
    test_release_train_forwards_release_context()
