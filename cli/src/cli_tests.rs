use super::*;
use crate::core::components::GenerationResult;
use crate::core::pipelines::InstallResult;

#[test]
fn render_result_handles_success_install_and_validation() {
    // Success (empty) — no visible output.
    render_result(CommandResult::Success(String::new()));

    // Success (non-empty) — prints message.
    render_result(CommandResult::Success("done".into()));

    // Install — prints summary (covers InstallResult::summary).
    let installed = InstallResult::success();
    render_result(CommandResult::Install(installed));

    // Generation — prints file count.
    let gen_res = GenerationResult {
        generated_files: vec![std::path::PathBuf::from("a.rs")],
        warnings: vec![],
        errors: vec![],
    };
    render_result(CommandResult::Generation(gen_res));

    // Validation — prints formatted report (covers ErrorReporter::format_validation_report
    // success-path summary).
    let report = crate::core::components::ValidationReport {
        is_valid: true,
        config_validation: crate::core::components::ConfigValidation {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        },
        dependency_validation: vec![],
        network_validation: vec![],
        fingerprint_validation: vec![],
        conflicts: vec![],
    };
    render_result(CommandResult::Validation(report));
}
