use std::collections::HashSet;

use edge_spike::corpus::{FailureClass, default_corpus, default_corpus_with_summary};
use edge_spike::runner::{RunSelector, RunnerConfig, run_corpus};

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

        // C1 audit: 14 trace-analyst edges are honest blockers.
        let (blocked_cal, blocked_held) = counts
            .get(&FailureClass::BaselineBlocked)
            .copied()
            .unwrap_or((0, 0));
        if blocked_cal + blocked_held != 14 {
            anyhow::bail!(
                "expected 14 BaselineBlocked variants, got {}",
                blocked_cal + blocked_held
            );
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

    #[test]
    fn baseline_blocked_are_trace_analyst_iter1_task1() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        let blocked: Vec<_> = corpus
            .calibration
            .iter()
            .chain(corpus.held_out.iter())
            .filter(|e| e.class == FailureClass::BaselineBlocked)
            .collect();
        assert_eq!(blocked.len(), 14, "expected 14 BaselineBlocked variants");
        for edge in &blocked {
            if edge.edge.worker_id != "trace-analyst" {
                anyhow::bail!(
                    "BaselineBlocked edge {} has worker_id {}, expected trace-analyst",
                    edge.variant_id,
                    edge.edge.worker_id
                );
            }
            // The parser stores iterations zero-based: internal iteration 0
            // corresponds to the audit's "iteration 1".
            if edge.edge.iteration != 0 || edge.edge.task_id != 1 {
                anyhow::bail!(
                    "BaselineBlocked edge {} is internal iteration {} task {}, expected internal iteration 0 task 1",
                    edge.variant_id,
                    edge.edge.iteration,
                    edge.edge.task_id
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn smoke_selector_is_deterministic() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        let config = RunnerConfig {
            selector: RunSelector::Smoke,
            ..RunnerConfig::default()
        };

        let result1 = run_corpus(&corpus, &config, None).await?;
        let result2 = run_corpus(&corpus, &config, None).await?;

        let id1 = result1.scored.first().map(|s| s.variant_id.as_str());
        let id2 = result2.scored.first().map(|s| s.variant_id.as_str());
        assert_eq!(id1, id2, "--smoke must select the same variant twice");
        Ok(())
    }

    #[tokio::test]
    async fn slice_selector_is_deterministic() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        let config = RunnerConfig {
            selector: RunSelector::Slice(12),
            ..RunnerConfig::default()
        };

        let result1 = run_corpus(&corpus, &config, None).await?;
        let result2 = run_corpus(&corpus, &config, None).await?;

        let ids1: Vec<_> = result1
            .scored
            .iter()
            .map(|s| s.variant_id.clone())
            .collect();
        let ids2: Vec<_> = result2
            .scored
            .iter()
            .map(|s| s.variant_id.clone())
            .collect();
        assert_eq!(ids1, ids2, "--slice must be deterministic");

        // 12 clean variants across all selected arms.
        let distinct: HashSet<_> = result1
            .scored
            .iter()
            .map(|s| s.variant_id.clone())
            .collect();
        assert_eq!(
            distinct.len(),
            12,
            "--slice 12 must select 12 distinct variants"
        );

        // Round-robin produces one clean variant per scenario per pass.
        let scenarios: HashSet<_> = result1.scored.iter().map(|s| s.class).collect();
        assert!(
            scenarios.iter().all(|&c| c == FailureClass::Clean),
            "--slice must select only Clean originals"
        );
        Ok(())
    }
}
