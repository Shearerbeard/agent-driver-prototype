use std::collections::HashSet;

use edge_spike::corpus::{FailureClass, default_corpus, default_corpus_with_summary};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_stats_sanity() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        let edges = corpus.total() / 11;
        if edges != 147 {
            anyhow::bail!("expected 147 captured edges, got {edges}");
        }
        if corpus.total() != 1617 {
            anyhow::bail!("expected 1617 total variants, got {}", corpus.total());
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

    #[test]
    fn corpus_build_summary_is_sane() -> anyhow::Result<()> {
        let (corpus, summary) = default_corpus_with_summary()?;
        assert_eq!(corpus.total(), summary.total_variants);
        assert_eq!(corpus.calibration.len(), summary.calibration_variants);
        assert_eq!(corpus.held_out.len(), summary.held_out_variants);
        assert_eq!(summary.chimeras_eliminated, 42);
        assert_eq!(summary.submit_result_count + summary.fallback_count, 147);
        assert!(summary.orphan_tool_calls < 10);
        Ok(())
    }
}
