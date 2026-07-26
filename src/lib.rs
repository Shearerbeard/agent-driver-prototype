//! Spike prototype: aura orchestration prompt-frame machinery ported onto
//! agent-driver-rs, verified byte-for-byte against the aura S2 golden
//! envelope corpus. Card S70 in terminalbench-aura.

pub mod config_builders;
pub mod context;
pub mod corpus_configuration;
pub mod fixture;
pub mod message;
pub mod producers;
pub mod prompt_constants;
pub mod templates;
pub mod types;
