//! Offline policy-change instrument.
//!
//! Given a run directory containing `corpus.json` + `results.jsonl`, rebuild
//! each saved row's state deterministically from the corpus (the same code
//! path as live), recompute the truncation flags, and recompute composite +
//! verdict from the SAVED answers. The scorer is never called. Writes
//! `results-rescored.jsonl` and prints an old-vs-new delta table.
//!
//!   cargo run -p edge-spike --example rescore -- <run-dir>

use edge_spike::runner::{RescoreOutcome, rescore_run_dir};
use edge_spike::verdict::{TruncationFlags, Verdict};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let run_dir = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: rescore <run-dir>"))?;

    let outcome = rescore_run_dir(std::path::Path::new(&run_dir)).await?;
    print_table(&outcome);
    println!("\nwrote {}", outcome.written.display());
    Ok(())
}

fn print_table(outcome: &RescoreOutcome) {
    println!("| edge | arm | old verdict | new verdict | old flags | new flags |");
    println!("| --- | --- | --- | --- | --- | --- |");
    for delta in &outcome.deltas {
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            delta.variant_id,
            delta.arm.name(),
            verdict_label(&delta.old_verdict),
            verdict_label(&delta.new_verdict),
            flags_label(&delta.old_flags),
            flags_label(&delta.new_flags),
        );
    }

    let flips = outcome
        .deltas
        .iter()
        .filter(|d| d.old_verdict != d.new_verdict)
        .count();
    println!("\n{} rows, {} verdict changes", outcome.deltas.len(), flips);
}

fn verdict_label(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Pass => "PASS".to_owned(),
        Verdict::Flag { dims } => format!("FLAG{{{}}}", dims.join(",")),
        Verdict::InsufficientEvidence => "INSUFFICIENT_EVIDENCE".to_owned(),
        Verdict::VerifierError => "VERIFIER_ERROR".to_owned(),
    }
}

/// Names of the true flags, in declaration order. `~source` and `~excerpted`
/// mark the two informational (never verdict-driving) flags.
fn flags_label(flags: &TruncationFlags) -> String {
    let mut names = Vec::new();
    if flags.goal {
        names.push("goal");
    }
    if flags.plan_step {
        names.push("plan_step");
    }
    if flags.plan_context {
        names.push("plan_context");
    }
    if flags.worker_output {
        names.push("worker_output");
    }
    if flags.context_provided {
        names.push("context_provided");
    }
    if flags.evidence {
        names.push("evidence");
    }
    if flags.evidence_excerpted {
        names.push("~excerpted");
    }
    if flags.source {
        names.push("~source");
    }
    if names.is_empty() {
        "-".to_owned()
    } else {
        names.join(",")
    }
}
