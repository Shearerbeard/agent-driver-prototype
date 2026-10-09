//! Dump the exact scorer state for one variant + arm from a corpus
//! artifact, with its hash, for offline inspection.
//!
//!   cargo run -p edge-spike --example dump-state -- \
//!     <corpus.json> <variant_id> <arm>

use edge_spike::arms::{ArmConfig, SelectionArm, assemble_state};
use edge_spike::corpus::Corpus;
use edge_spike::summarizer::FixtureSummarizer;
use edge_spike::verdict::hash_state;

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
    let json = serde_json::to_string_pretty(&state)?;
    println!(
        "variant: {variant_id} | class: {:?} | arm: {arm_name}",
        labelled.class
    );
    println!("state_hash: {}", hash_state(&state));
    println!("state_chars: {}", state.char_len());
    println!("{json}");
    Ok(())
}
