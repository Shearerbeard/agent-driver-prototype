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
#[derive(Debug, Clone)]
pub struct ArtifactsConfig {
    /// Optional base directory for execution persistence and plan storage.
    ///
    /// Structure: `<memory_dir>/<run_id>/iteration-{n}/...`
    /// If not set, execution persistence is disabled.
    pub memory_dir: Option<String>,
    /// Whether completed-task tool-chain lines render in the continuation
    /// prompt. Defaults to `false` (the accepted baseline).
    pub show_tool_reasoning_in_continuation: bool,
    /// Character cap for the inline result preview in continuation prompts.
    /// Defaults to 2000.
    pub result_summary_length: usize,
}

impl Default for ArtifactsConfig {
    fn default() -> Self {
        Self {
            memory_dir: None,
            show_tool_reasoning_in_continuation: false,
            result_summary_length: 2000,
        }
    }
}

// ============================================================================
// Orchestration Config
// ============================================================================

/// Orchestration configuration for specialized worker orchestration.
///
/// In orchestration mode, a coordinator agent decomposes queries into tasks executed by worker agents.
/// The coordinator's system prompt comes from `[agent].system_prompt`.
#[derive(Debug, Clone)]
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

    /// Maximum number of tool names to list per worker in the planning prompt
    /// before appending `(+N more)`.
    pub max_tools_per_worker: usize,

    // --- Sub-configs ---
    /// Artifact and persistence settings.
    pub artifacts: ArtifactsConfig,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_planning_cycles: 3,
            worker_system_prompt: None,
            workers: HashMap::new(),
            coordinator_vector_stores: Vec::new(),
            allow_direct_answers: true,
            allow_clarification: true,
            tools_in_planning: ToolVisibility::Summary,
            max_tools_per_worker: 10,
            artifacts: ArtifactsConfig::default(),
        }
    }
}

impl OrchestrationConfig {
    /// Whether orchestration mode has specialized workers configured.
    ///
    /// When true, tasks should be assigned to specific workers during planning.
    /// When false, all tasks use the generic worker preamble.
    pub fn has_workers(&self) -> bool {
        !self.workers.is_empty()
    }

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

    /// Whether completed-task tool-chain lines render in the continuation
    /// prompt.
    pub fn show_tool_reasoning_in_continuation(&self) -> bool {
        self.artifacts.show_tool_reasoning_in_continuation
    }

    /// Character cap for the inline result preview in continuation prompts.
    pub fn result_summary_length(&self) -> usize {
        self.artifacts.result_summary_length
    }

    /// Whether persistence (memory_dir) is enabled.
    pub fn memory_dir(&self) -> Option<&str> {
        self.artifacts.memory_dir.as_deref()
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

impl VectorStoreConfig {
    /// Construct with default fields except name and context_prefix.
    pub fn new(name: &str, context_prefix: Option<&str>) -> Self {
        Self {
            name: name.to_owned(),
            context_prefix: context_prefix.map(str::to_owned),
        }
    }
}

// ============================================================================
// Skill Configuration
// ============================================================================

/// A validated skill name (1-64 chars, lowercase alphanumerics and hyphens,
/// no leading/trailing/consecutive hyphens).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkillName(String);

impl SkillName {
    /// Parse a skill name, returning an error string on validation failure.
    pub fn new(name: impl Into<String>) -> Result<Self, String> {
        let name = name.into();
        validate_skill_name(&name)?;
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkillName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for SkillName {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for SkillName {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err(format!(
            "Skill name must be 1-64 characters, got {} characters",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "Skill name '{name}' contains invalid characters (only lowercase alphanumeric and hyphens allowed)"
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(format!(
            "Skill name '{name}' must not start or end with a hyphen"
        ));
    }
    if name.contains("--") {
        return Err(format!(
            "Skill name '{name}' must not contain consecutive hyphens"
        ));
    }
    Ok(())
}

/// Skill configuration for on-demand loading via the `load_skill` tool.
#[derive(Debug, Clone)]
pub struct SkillConfig {
    /// Unique name for this skill (must match directory name).
    pub name: SkillName,
    /// Human-readable description.
    pub description: String,
    /// Absolute path to the skill directory.
    pub path: std::path::PathBuf,
}

// ============================================================================
// Glob Matching
// ============================================================================

/// Match a glob pattern against a string.
///
/// Supports:
/// - `*` matches zero or more characters
/// - `?` matches exactly one character
///
/// Examples:
/// - `mezmo_*` matches `mezmo_logs`, `mezmo_pipelines`
/// - `*Query*` matches `ListQuery`, `QueryKnowledgeBases`
/// - `tool_?` matches `tool_a`, `tool_b`
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    fn match_recursive(pattern: &[char], text: &[char]) -> bool {
        match (pattern.first(), text.first()) {
            // Both exhausted - match!
            (None, None) => true,
            // Pattern exhausted but text remains - no match
            (None, Some(_)) => false,
            // Wildcard * - try matching zero or more characters
            (Some('*'), _) => {
                // Try matching zero characters (skip *)
                if match_recursive(&pattern[1..], text) {
                    return true;
                }
                // Try matching one character and continue with *
                if !text.is_empty() && match_recursive(pattern, &text[1..]) {
                    return true;
                }
                false
            }
            // Text exhausted but pattern has non-* remaining - check for trailing *s
            (Some(p), None) => *p == '*' && match_recursive(&pattern[1..], text),
            // Single character wildcard ?
            (Some('?'), Some(_)) => match_recursive(&pattern[1..], &text[1..]),
            // Literal character match
            (Some(p), Some(t)) => *p == *t && match_recursive(&pattern[1..], &text[1..]),
        }
    }

    match_recursive(&pattern, &text)
}
