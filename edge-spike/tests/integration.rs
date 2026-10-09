//! Integration tests for the edge-spike offline replay.

use edge_spike::accounting::{Accounting, AccuracySummary};
use edge_spike::arms::SelectionArm;
use edge_spike::claim::{extract_claims, resolve_claims};
use edge_spike::corpus::{FailureClass, default_corpus};
use edge_spike::report::generate_report;
use edge_spike::runner::{RunnerConfig, replay_corpus};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn corpus_parses_and_has_edges() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        if corpus.total() < 22 {
            return Err(anyhow::anyhow!(
                "expected at least 22 variants, got {}",
                corpus.total()
            ));
        }
        let counts = corpus.class_counts();
        for class in FailureClass::all() {
            let (cal, held) = counts.get(class).copied().unwrap_or((0, 0));
            if cal + held == 0 {
                return Err(anyhow::anyhow!(
                    "class {class:?} should have at least one variant"
                ));
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn replay_runs_all_arms() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        let config = RunnerConfig::default();
        let scored = replay_corpus(&corpus, &config).await?;
        let expected = corpus.total() * SelectionArm::all().len();
        if scored.len() != expected {
            return Err(anyhow::anyhow!(
                "expected {expected} scored edges, got {}",
                scored.len()
            ));
        }
        Ok(())
    }

    #[tokio::test]
    async fn report_reproduces_from_scored() -> anyhow::Result<()> {
        let corpus = default_corpus()?;
        let config = RunnerConfig::default();
        let scored = replay_corpus(&corpus, &config).await?;
        let accounting = Accounting::from_scored(&scored);
        let accuracy = AccuracySummary::from_scored(&scored);
        let report = generate_report(&corpus, &scored, &accounting, &accuracy);
        if !report.contains("W7 JEV Rubric Research Spike Report") {
            return Err(anyhow::anyhow!("report missing title"));
        }
        if !report.contains("claim_indexed") {
            return Err(anyhow::anyhow!("report missing claim_indexed arm"));
        }
        Ok(())
    }

    #[test]
    fn claim_extractor_finds_classes() {
        let text = r#"The incident id "inc-42" started at 2026-07-17T22:08:25Z on host "web-03.prod.internal" with 99.9% failure rate and 150ms latency."#;
        let claims = extract_claims(text);
        assert!(!claims.is_empty());
        let classes: std::collections::HashSet<_> = claims.iter().map(|c| c.class).collect();
        assert!(classes.contains(&edge_spike::claim::ClaimClass::QuotedString));
        assert!(classes.contains(&edge_spike::claim::ClaimClass::Timestamp));
        assert!(classes.contains(&edge_spike::claim::ClaimClass::ValueWithUnit));
    }

    #[test]
    fn claim_resolution_matches_capture_and_context() {
        let text = "The value was 150ms and the host was web-03.";
        let claims = extract_claims(text);
        let capture = "web-03 logged 150ms latency";
        let context = "prior task noted 150ms";
        let (resolved, stats) = resolve_claims(&claims, capture, context);
        assert!(!resolved.is_empty());
        assert!(stats.found_in_tool_capture + stats.found_in_context > 0);
    }
}
