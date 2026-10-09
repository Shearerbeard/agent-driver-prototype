//! edge-run: build corpus, run all arms, write artifacts.
//!
//! Usage:
//!   cargo run -p edge-spike --bin edge-run [artifact-dir]
//!   cargo run -p edge-spike --bin edge-run -- --live [artifact-dir]
//!   cargo run -p edge-spike --bin edge-run -- --live --local-gateway [artifact-dir]
//!   cargo run -p edge-spike --bin edge-run -- --live --dry-run [artifact-dir]

use std::env;
use std::path::PathBuf;

use edge_spike::client::JevEndpoint;
use edge_spike::corpus::default_corpus;
use edge_spike::runner::{
    RunnerConfig, RunnerMode, plan_run, write_artifacts, write_judge_results,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = env::args().skip(1).peekable();

    let mut live = env::var("LIVE").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let mut local_gateway = false;
    let mut dry_run = false;
    let mut artifact_dir: Option<PathBuf> = None;

    for arg in args {
        match arg.as_str() {
            "--live" => live = true,
            "--local-gateway" => local_gateway = true,
            "--dry-run" => dry_run = true,
            other => {
                if other.starts_with('-') {
                    anyhow::bail!("unknown flag: {other}");
                }
                artifact_dir = Some(PathBuf::from(other));
            }
        }
    }

    let artifact_dir = artifact_dir.unwrap_or_else(|| PathBuf::from("edge-spike-artifacts"));

    let mut config = RunnerConfig::default();
    if live {
        config.mode = RunnerMode::Live;
        config.endpoint = if local_gateway {
            JevEndpoint::LocalGateway
        } else {
            JevEndpoint::Cloud
        };
    }

    let corpus = default_corpus()?;

    if dry_run {
        let plan = plan_run(&corpus, &config);
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
