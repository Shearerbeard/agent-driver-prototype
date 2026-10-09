//! Dump the exact wire request (state + questions) for one variant + arm.
//!
//!   cargo run -p edge-spike --example dump-request -- \
//!     <corpus.json> <variant_id> <arm>

use edge_spike::arms::{ArmConfig, SelectionArm, assemble_state};
use edge_spike::corpus::Corpus;
use edge_spike::rubric::EdgeRubric;
use edge_spike::summarizer::FixtureSummarizer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let corpus_path = args.next().expect("corpus.json path");
    let variant_id = args.next().expect("variant id");
    let arm_name = args.next().expect("arm name");

    let corpus: Corpus = serde_json::from_str(&std::fs::read_to_string(corpus_path)?)?;
    let labelled = corpus
        .calibration
        .iter()
        .chain(corpus.held_out.iter())
        .find(|e| e.variant_id == variant_id)
        .ok_or_else(|| anyhow::anyhow!("variant not found: {variant_id}"))?;
    let arm = SelectionArm::all()
        .iter()
        .copied()
        .find(|a| a.name() == arm_name)
        .ok_or_else(|| anyhow::anyhow!("unknown arm: {arm_name}"))?;

    let config = ArmConfig::default();
    let state = assemble_state(&labelled.edge, arm, &config, &FixtureSummarizer).await?;
    let decision = EdgeRubric::request().state(&state)?.build()?;
    let body = serde_json::json!({
        "state": decision.state(),
        "model": "jev-1.13.0",
        "questions": decision.questions(),
    });
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}
