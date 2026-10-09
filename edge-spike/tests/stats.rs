use std::collections::HashSet;

use edge_spike::corpus::{FailureClass, default_corpus};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_stats_sanity() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        let edges = corpus.total() / 11;
        if edges < 40 {
            anyhow::bail!("expected at least 40 captured edges, got {edges}");
        }
        if corpus.total() != edges * 11 {
            anyhow::bail!(
                "total variants {} is not a multiple of 11 for {edges} edges",
                corpus.total()
            );
        }

        let mut ids = HashSet::new();
        for edge in corpus.calibration.iter().chain(corpus.held_out.iter()) {
            if !ids.insert(edge.variant_id.clone()) {
                anyhow::bail!("duplicate variant_id {}", edge.variant_id);
            }
        }

        let counts = corpus.class_counts();
        for class in FailureClass::all() {
            let (cal, held) = counts.get(class).copied().unwrap_or((0, 0));
            if cal == 0 {
                anyhow::bail!("class {class:?} missing in calibration");
            }
            if held == 0 {
                anyhow::bail!("class {class:?} missing in held-out");
            }
        }
        Ok(())
    }
}
