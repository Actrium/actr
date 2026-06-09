#!/usr/bin/env python3

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CI_GATE_WORKFLOW = ROOT / ".github/workflows/ci-gate.yml"
CI_E2E_WORKFLOW = ROOT / ".github/workflows/ci-e2e.yml"


def _job(workflow: str, name: str, next_name: str) -> str:
    job_start = workflow.index(f"  {name}:\n")
    next_job_start = workflow.index(f"\n  {next_name}:\n", job_start)
    return workflow[job_start:next_job_start]


def test_rust_gate_avoids_slow_workspace_tests_and_unused_prewarm() -> None:
    workflow = CI_GATE_WORKFLOW.read_text(encoding="utf-8")
    rust_job = _job(workflow, "rust", "typescript")

    assert "- name: Run tests" not in rust_job
    assert "cargo test --workspace" not in rust_job
    assert "- name: Prepare Rust codegen plugins" not in rust_job
    assert "cargo install protoc-gen-prost" not in rust_job
    assert "cargo build --quiet -p actr-framework-protoc-codegen" not in rust_job


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


if __name__ == "__main__":
    test_rust_gate_avoids_slow_workspace_tests_and_unused_prewarm()
    test_pr_gate_excludes_heavy_root_e2e_jobs()
    test_scheduled_e2e_runs_root_level_browser_and_stream_e2e()
