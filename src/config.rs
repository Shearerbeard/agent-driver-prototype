//! Minimal local mirror of the `aura_config` orchestration types touched by
//! the ported preamble builders and roster rendering.
//!
//! Only the fields and methods the ported code reads are carried here. The
//! full serializable types live in `aura_config::orchestration`; this spike
//! mirrors the surface the Phase 1 port exercises (preamble building, worker
//! roster formatting, and the prompt-rendering QA test).

use std::collections::HashMap;

// ============================================================================
// Tool Visibility Configuration
// ============================================================================

/// Controls how tool information is shown to the coordinator during planning.
///
/// This is **display only** — it does not affect which tools workers can execute.
/// Tool execution access is controlled by each worker's `mcp_filter`.
/// This setting only affects what the coordinator sees when deciding how to
/// assign tasks, balancing context length vs. precision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ToolVisibility {
    /// No tool information in planning prompt (minimal context, display only).
    None,
    /// Tool names only, bucketed by worker (default — good balance, display only).
    #[default]
    Summary,
    /// Tool names with descriptions (maximum context, higher token usage, display only).
    Full,
}

// ============================================================================
// Per-Worker Configuration
// ============================================================================

/// Per-worker configuration for specialized workers.
///
/// Workers are specialized agents with custom preambles and filtered tool access.
/// Configure workers using TOML sections like `[orchestration.worker.operations]`.
///
/// # Example
///
/// ```toml
/// [orchestration.worker.operations]
/// description = "For logs, pipelines, metrics, and system analysis"
/// preamble = "You are an Operations Specialist..."
/// mcp_filter = ["mezmo_*"]
/// ```
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Short description of this worker's purpose (for planning prompt).
    ///
    /// This is shown to the LLM during planning so it can assign tasks appropriately.
    /// Keep it concise (one line).
    pub description: String,

    /// System prompt for this worker (replaces generic worker preamble).
    ///
    /// This is the complete system prompt - it does NOT use the worker_preamble.md template.
    /// For specialized workers, provide domain-specific instructions here.
    pub preamble: String,

    /// Glob patterns for which MCP tools this worker gets access to.
    ///
    /// Examples:
    /// - `["mezmo_*"]` - all tools starting with "mezmo_"
    /// - `["ListKnowledgeBases", "QueryKnowledgeBases"]` - specific tools
    /// - `["*"]` or empty - all tools (default)
    ///
    /// Patterns are matched using glob syntax (supports `*`, `**`, `?`, `[abc]`).
    pub mcp_filter: Vec<String>,

    /// Vector stores this worker has access to.
    ///
    /// By default (empty), workers have NO vector store access. Workers must
    /// explicitly list the stores they need. This prevents unintended RAG
    /// access and keeps workers focused on their specialization.
    ///
    /// Values should match the `name` field of entries in `[[vector_stores]]`.
    pub vector_stores: Vec<String>,

    /// Max tool-calling turns for this worker.
    ///
    /// Controls how many Rig ReAct turns (tool calls) a worker can make
    /// per task execution. Overrides `[agent].turn_depth`. Falls back to
    /// `[agent].turn_depth` → `DEFAULT_MAX_DEPTH` if not set.
    pub turn_depth: Option<usize>,

    /// Optional per-worker LLM override.
    ///
    /// When `Some`, the worker runs with this LLM config instead of inheriting
    /// `[agent.llm]`. The resolved `context_window` drives per-worker budget
    /// math (e.g. scratchpad sizing).
    pub llm: Option<LlmConfig>,

    /// Per-worker override of `[agent.scratchpad]`. Parsed from
    /// `[orchestration.worker.<name>.scratchpad]`.
    pub scratchpad: Option<ScratchpadConfig>,

    /// Per-worker skill sources. Parsed from `[orchestration.worker.<name>.skills]`.
    /// `None` inherits `[agent.skills]`; an explicit empty list disables skills
    /// for this worker; a non-empty list replaces the agent's skills entirely
    /// (no merging).
    pub skills: Option<SkillsConfig>,
}

/// Placeholder for the per-worker LLM override config (not read by ported code).
#[derive(Debug, Clone, Default)]
pub struct LlmConfig;

/// Placeholder for the per-worker scratchpad config (not read by ported code).
#[derive(Debug, Clone, Default)]
pub struct ScratchpadConfig;

/// Placeholder for the per-worker skills config (not read by ported code).
#[derive(Debug, Clone, Default)]
pub struct SkillsConfig;

// ============================================================================
// Artifacts Sub-Config
// ============================================================================

/// Artifact and persistence configuration for orchestration.
///
/// # Example
///
/// ```toml
/// [orchestration.artifacts]
/// memory_dir = "/tmp/aura-orchestration"
/// result_artifact_threshold = 4000
/// result_summary_length = 2000
/// ```
#[derive(Debug, Clone, Default)]
pub struct ArtifactsConfig {
    /// Optional base directory for execution persistence and plan storage.
    ///
    /// Structure: `<memory_dir>/<run_id>/iteration-{n}/...`
    /// If not set, execution persistence is disabled.
    pub memory_dir: Option<String>,
}

// ============================================================================
// Orchestration Config
// ============================================================================

/// Orchestration configuration for specialized worker orchestration.
///
/// In orchestration mode, a coordinator agent decomposes queries into tasks executed by worker agents.
/// The coordinator's system prompt comes from `[agent].system_prompt`.
#[derive(Debug, Clone, Default)]
pub struct OrchestrationConfig {
    // --- Mode ---
    /// Whether orchestration mode is enabled.
    /// When false (default), standard single-agent streaming is used.
    pub enabled: bool,

    // --- Planning loop ---
    /// Maximum number of plan-execute-continue cycles.
    pub max_planning_cycles: usize,

    // --- Worker defaults ---
    /// Custom system prompt to inject into worker agents.
    pub worker_system_prompt: Option<String>,

    /// Specialized worker configurations.
    pub workers: HashMap<String, WorkerConfig>,

    // --- Coordinator ---
    /// Vector stores available to the coordinator agent.
    pub coordinator_vector_stores: Vec<String>,

    // --- Routing ---
    /// Allow the coordinator to answer simple queries directly without orchestration.
    pub allow_direct_answers: bool,

    /// Allow the coordinator to request clarification for ambiguous queries.
    pub allow_clarification: bool,

    // --- Planning display ---
    /// Controls how tool information is shown to the coordinator during planning.
    pub tools_in_planning: ToolVisibility,

    // --- Sub-configs ---
    /// Artifact and persistence settings.
    pub artifacts: ArtifactsConfig,
}

impl OrchestrationConfig {
    /// Get a worker configuration by name.
    ///
    /// Returns `None` if the worker doesn't exist.
    pub fn get_worker(&self, name: &str) -> Option<&WorkerConfig> {
        self.workers.get(name)
    }

    /// Get the names of all configured workers.
    ///
    /// Used to include available workers in the planning prompt.
    pub fn available_worker_names(&self) -> Vec<&str> {
        self.workers.keys().map(|s| s.as_str()).collect()
    }

    /// Format worker descriptions for the planning prompt.
    ///
    /// Returns a formatted string listing all workers with their descriptions.
    /// Example output:
    /// ```text
    /// - operations: For logs, pipelines, metrics, and system analysis
    /// - knowledge: For documentation, procedures, and best practices
    /// ```
    pub fn format_workers_for_prompt(&self) -> String {
        if self.workers.is_empty() {
            return String::new();
        }

        self.workers
            .iter()
            .map(|(name, config)| format!("- {}: {}", name, config.description))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ============================================================================
// Vector Store Configuration
// ============================================================================

/// Vector store configuration (mirror — only the fields the ported
/// `build_vector_store_context` helper reads).
#[derive(Debug, Clone)]
pub struct VectorStoreConfig {
    /// Unique name to identify this vector store
    pub name: String,
    /// Optional context string describing what the vector store contains (for better LLM guidance)
    pub context_prefix: Option<String>,
}
