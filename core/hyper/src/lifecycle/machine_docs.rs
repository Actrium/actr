//! Canonical, deterministic documentation generator for the lifecycle state
//! machines (RFC-0400 "Executable state-machine reference").
//!
//! YASM 0.6.0 builds its Mermaid groups with hash maps and does not promise raw
//! output order, so checking its bytes against the RFC's marked regions is
//! invalid. This generator re-derives both artifacts with a stable order:
//!
//! - [`canonical_transition_table`] sorts transition tuples by
//!   `(current_state, input, next_state)` and strips YASM's document-level
//!   table heading;
//! - [`canonical_mermaid`] groups equal `(current_state, next_state)` pairs and
//!   sorts their input labels, grouping through a sorted `Vec` rather than a
//!   hash map.
//!
//! Entry points are provided for all eight normative machines: the three new
//! policy machines, plus the five pre-RFC machines referenced directly from
//! their existing modules.
//!
//! The generator is a documentation and CI tool by design: it is consumed by
//! tests and the RFC's checked regeneration mode, not by any runtime path. The
//! module-level `allow(dead_code)` keeps the per-machine entry points available
//! for that regeneration even when a given build does not call all of them.
#![allow(dead_code)]

use yasm::StateMachine;

use super::connection_supervisor::{app_phase, offline_work, path, recovery};
use super::recovery_execution::RecoveryExecutionMachine;
use super::recovery_policy::machines::{cleanup_work, recovery_mode, retry_gate};

/// Generate a transition table sorted by `(current_state, input, next_state)`.
///
/// The output omits YASM's `# State Transition Table` heading and uses the
/// RFC's compact column separator, so it can replace a marked region verbatim.
pub(crate) fn canonical_transition_table<SM: StateMachine>() -> String {
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for state in SM::states() {
        for input in SM::valid_inputs(&state) {
            if SM::input_name(&input).starts_with('_') {
                continue;
            }
            if let Some(next) = SM::next_state(&state, &input) {
                rows.push((
                    SM::state_name(&state),
                    SM::input_name(&input),
                    SM::state_name(&next),
                ));
            }
        }
    }
    rows.sort();

    let mut out = String::from("| Current State | Input | Next State |\n|---|---|---|\n");
    for (state, input, next) in rows {
        out.push_str(&format!("| {state} | {input} | {next} |\n"));
    }
    out
}

/// Generate a Mermaid state diagram with a deterministic order.
///
/// Transitions are collected into a `Vec`, sorted by
/// `(current_state, next_state, input)`, and then walked to group consecutive
/// equal `(current_state, next_state)` pairs. Because the vector is sorted, the
/// input labels within each group are already ordered and the groups themselves
/// appear in a stable order — no hash-map iteration is involved.
pub(crate) fn canonical_mermaid<SM: StateMachine>() -> String {
    let mut edges: Vec<(String, String, String)> = Vec::new();
    for state in SM::states() {
        for input in SM::valid_inputs(&state) {
            if SM::input_name(&input).starts_with('_') {
                continue;
            }
            if let Some(next) = SM::next_state(&state, &input) {
                edges.push((
                    SM::state_name(&state),
                    SM::state_name(&next),
                    SM::input_name(&input),
                ));
            }
        }
    }
    edges.sort();

    let mut out = String::from("stateDiagram-v2\n");
    out.push_str(&format!(
        "    [*] --> {}\n",
        SM::state_name(&SM::initial_state())
    ));

    let mut index = 0;
    while index < edges.len() {
        let from = edges[index].0.clone();
        let to = edges[index].1.clone();
        let mut labels: Vec<String> = Vec::new();
        while index < edges.len() && edges[index].0 == from && edges[index].1 == to {
            labels.push(edges[index].2.clone());
            index += 1;
        }
        out.push_str(&format!("    {from} --> {to} : {}\n", labels.join(" / ")));
    }
    out
}

/// The canonical documentation for one machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalDoc {
    pub name: &'static str,
    pub table: String,
    pub mermaid: String,
}

fn doc<SM: StateMachine>(name: &'static str) -> CanonicalDoc {
    CanonicalDoc {
        name,
        table: canonical_transition_table::<SM>(),
        mermaid: canonical_mermaid::<SM>(),
    }
}

/// The canonical documentation for all eight normative machines, in a stable
/// order. The three new policy machines come first, then the five pre-RFC
/// machines referenced from their existing modules.
pub(crate) fn all_machine_docs() -> Vec<CanonicalDoc> {
    vec![
        doc::<recovery_mode::RecoveryModeMachine>("recovery-mode"),
        doc::<cleanup_work::CleanupWorkMachine>("cleanup-work"),
        doc::<retry_gate::RetryGateMachine>("retry-gate"),
        doc::<app_phase::AppPhaseMachine>("app-phase"),
        doc::<path::NetworkPathMachine>("network-path"),
        doc::<recovery::RecoveryIntentMachine>("recovery-intent"),
        doc::<offline_work::OfflineWorkMachine>("offline-work"),
        doc::<RecoveryExecutionMachine>("recovery-execution"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_machines_have_canonical_docs() {
        let docs = all_machine_docs();
        assert_eq!(docs.len(), 8);
        for d in &docs {
            assert!(d.mermaid.starts_with("stateDiagram-v2\n"), "{}", d.name);
            assert!(
                d.table
                    .starts_with("| Current State | Input | Next State |"),
                "{}",
                d.name
            );
            // The document-level YASM heading must be stripped.
            assert!(!d.table.contains("# State Transition Table"), "{}", d.name);
        }
    }

    #[test]
    fn generation_is_byte_stable_across_calls() {
        // Two calls in the same process must produce identical bytes despite
        // YASM's internal hash-map grouping.
        for _ in 0..8 {
            assert_eq!(
                canonical_transition_table::<recovery_mode::RecoveryModeMachine>(),
                canonical_transition_table::<recovery_mode::RecoveryModeMachine>()
            );
            assert_eq!(
                canonical_mermaid::<path::NetworkPathMachine>(),
                canonical_mermaid::<path::NetworkPathMachine>()
            );
        }
        assert_eq!(all_machine_docs(), all_machine_docs());
    }

    #[test]
    fn transition_table_rows_are_sorted() {
        let table = canonical_transition_table::<retry_gate::RetryGateMachine>();
        let rows: Vec<&str> = table
            .lines()
            .filter(|l| l.starts_with("| ") && !l.contains("---") && !l.contains("Current State"))
            .collect();
        let mut sorted = rows.clone();
        sorted.sort();
        assert_eq!(rows, sorted, "rows must be lexicographically sorted");
        // The recovery-mode machine's first sorted row is deterministic.
        let mode = canonical_transition_table::<recovery_mode::RecoveryModeMachine>();
        let first = mode
            .lines()
            .find(|l| l.starts_with("| Active"))
            .expect("Active row present");
        assert_eq!(first, "| Active | AppTerminating | Terminating |");
    }

    #[test]
    fn mermaid_group_input_labels_are_sorted() {
        // Terminating self-loops on all three inputs, sorted alphabetically.
        let mermaid = canonical_mermaid::<recovery_mode::RecoveryModeMachine>();
        assert!(
            mermaid.contains("    Terminating --> Terminating : AppTerminating / SessionActivated / UserLoggedOut\n"),
            "self-loop labels must be sorted:\n{mermaid}"
        );
        assert!(mermaid.contains("    [*] --> Active\n"));
    }
}
