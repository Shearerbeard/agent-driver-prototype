//! Spike prototype: aura orchestration prompt-frame machinery ported onto
//! agent-driver-rs, verified byte-for-byte against the aura S2 golden
//! envelope corpus. Card S70 in terminalbench-aura.

pub mod artifacts;
pub mod bounding;
pub mod config;
pub mod config_builders;
pub mod context;
pub mod coordinator_loop;
pub mod corpus_configuration;
pub mod dag_executor;
#[cfg(test)]
pub mod fixture;
pub mod mcp_client;
pub mod message;
pub mod persistence;
pub mod producers;
pub mod prompt_constants;
pub mod shim_config;
pub mod sse_shim;
pub mod templates;
pub mod tools;
pub mod types;

#[cfg(test)]
mod golden_tests;
