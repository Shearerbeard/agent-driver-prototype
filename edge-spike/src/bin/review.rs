//! Review script scaffold: reproduces reported numbers from raw artifacts.
//!
//! Run after the offline replay to regenerate the report from the
//! artifact directory.

use std::path::PathBuf;

use edge_spike::accounting::{Accounting, AccuracySummary};
use edge_spike::corpus::Corpus;
use edge_spike::report::generate_report;
use edge_spike::runner::ScoredEdge;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let artifact_dir = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("edge-spike-artifacts"));

    let corpus_path = artifact_dir.join("corpus.json");
    let corpus: Corpus = serde_json::from_str(&std::fs::read_to_string(corpus_path)?)?;

    let mut scored = Vec::new();
    for entry in std::fs::read_dir(&artifact_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name();
        if path.extension().is_some_and(|e| e == "json")
            && file_name != Some("corpus.json".as_ref())
            && file_name != Some("report.json".as_ref())
            && file_name != Some("judge_results.json".as_ref())
            && file_name != Some("resume.json".as_ref())
        {
            let edge: ScoredEdge = serde_json::from_str(&std::fs::read_to_string(path)?)?;
            scored.push(edge);
        }
    }

    let accounting = Accounting::from_scored(&scored);
    let accuracy = AccuracySummary::from_scored(&scored);
    let report = generate_report(&corpus, &scored, &accounting, &accuracy);
    println!("{report}");

    Ok(())
}
