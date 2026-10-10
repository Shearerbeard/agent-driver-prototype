//! edge-run: build corpus, run all arms, write artifacts.
//!
//! Usage:
//!   cargo run -p edge-spike --bin edge-run [artifact-dir]
//!   cargo run -p edge-spike --bin edge-run -- --live [artifact-dir]
//!   cargo run -p edge-spike --bin edge-run -- --live --local-gateway [artifact-dir]
//!   cargo run -p edge-spike --bin edge-run -- --live --dry-run [artifact-dir]
//!   cargo run -p edge-spike --bin edge-run -- --live --judge [artifact-dir]

use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

use edge_spike::arms::SelectionArm;
use edge_spike::client::JevEndpoint;
use edge_spike::corpus::default_corpus;
use edge_spike::runner::{
    RunSelector, RunnerConfig, RunnerMode, plan_run, write_artifacts, write_judge_results,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1).peekable();

    let mut live = env::var("LIVE").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let mut local_gateway = false;
    let mut dry_run = false;
    let mut judge = false;
    let mut selector = RunSelector::All;
    let mut arms: Option<Vec<SelectionArm>> = None;
    let mut artifact_dir: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--live" => live = true,
            "--local-gateway" => local_gateway = true,
            "--dry-run" => dry_run = true,
            "--judge" => judge = true,
            "--edge" => {
                let id = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--edge requires a variant id"))?;
                selector = RunSelector::Edge(id);
            }
            "--smoke" => selector = RunSelector::Smoke,
            "--slice" => {
                let n_str = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--slice requires N"))?;
                let n: usize = n_str
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--slice N must be a positive integer"))?;
                selector = RunSelector::Slice(n);
            }
            "--arms" => {
                let s = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--arms requires a comma-separated list"))?;
                arms = Some(parse_arms(&s)?);
            }
            other => {
                if other.starts_with('-') {
                    anyhow::bail!("unknown flag: {other}");
                }
                artifact_dir = Some(PathBuf::from(other));
            }
        }
    }
    selector.validate()?;

    let artifact_dir = artifact_dir.unwrap_or_else(|| PathBuf::from("edge-spike-artifacts"));

    let arms = arms.unwrap_or_else(|| SelectionArm::all().to_vec());
    if arms.is_empty() {
        anyhow::bail!("--arms list cannot be empty");
    }

    let mode = if live {
        RunnerMode::Live
    } else {
        RunnerMode::Replay
    };
    let endpoint = if local_gateway {
        JevEndpoint::LocalGateway
    } else {
        JevEndpoint::Cloud
    };

    let config = RunnerConfig {
        selector,
        arms,
        mode,
        endpoint,
        judge,
        ..RunnerConfig::default()
    };

    let corpus = default_corpus()?;

    if dry_run {
        let plan = plan_run(&corpus, &config)?;
        println!("{}", plan.format());
        return Ok(());
    }

    if live {
        let result = edge_spike::runner::run_corpus(&corpus, &config, Some(&artifact_dir)).await?;
        std::fs::create_dir_all(&artifact_dir)?;
        std::fs::write(
            artifact_dir.join("corpus.json"),
            serde_json::to_string_pretty(&corpus)?,
        )?;
        write_artifacts(&result.scored, &artifact_dir)?;
        write_judge_results(&result.judge_results, &artifact_dir)?;

        if let Some(resume) = result.resume {
            resume.write(&artifact_dir)?;
            anyhow::bail!("{}", resume.error);
        }

        println!(
            "wrote {} scored edges and {} judge results to {}",
            result.scored.len(),
            result.judge_results.len(),
            artifact_dir.display()
        );
    } else {
        let result = edge_spike::runner::run_corpus(&corpus, &config, Some(&artifact_dir)).await?;
        std::fs::create_dir_all(&artifact_dir)?;
        std::fs::write(
            artifact_dir.join("corpus.json"),
            serde_json::to_string_pretty(&corpus)?,
        )?;
        write_artifacts(&result.scored, &artifact_dir)?;

        if let Some(resume) = result.resume {
            resume.write(&artifact_dir)?;
            anyhow::bail!("{}", resume.error);
        }

        println!(
            "wrote {} scored edges to {}",
            result.scored.len(),
            artifact_dir.display()
        );
    }

    Ok(())
}

fn parse_arms(s: &str) -> anyhow::Result<Vec<SelectionArm>> {
    let mut arms = Vec::new();
    for token in s.split(',') {
        let token = token.trim();
        match token {
            "1" => arms.push(SelectionArm::HeadTruncation),
            "2" => arms.push(SelectionArm::ClaimIndexed),
            "3" => arms.push(SelectionArm::FlashSummarization),
            _ => anyhow::bail!(
                "unknown arm '{token}', expected 1 (head_truncation), 2 (claim_indexed), or 3 (flash_summarization)"
            ),
        }
    }
    if arms.is_empty() {
        anyhow::bail!("--arms list cannot be empty");
    }
    // Deduplicate while preserving order.
    let mut seen = HashSet::new();
    arms.retain(|a| seen.insert(*a));
    Ok(arms)
}
