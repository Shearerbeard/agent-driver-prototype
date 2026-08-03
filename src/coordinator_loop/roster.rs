//! The set of workers a plan may assign work to, carrying the renderable
//! per-worker spec the roster section needs.
//!
//! The roster is what makes worker assignment checkable: the planning schema
//! offers exactly these names and the plan parse rejects anything else, so a
//! plan can never be dispatched to a worker that does not exist. It also
//! carries the per-worker description and resolved tool list plus the
//! tool-visibility inputs, so [`WorkerSections::from_roster`](super::driver::WorkerSections::from_roster)
//! can render the roster section from the typed roster alone.

use crate::bounding::ToolListLimit;
use crate::config::{OrchestrationConfig, ToolVisibility, VectorStoreConfig};
use crate::context::WorkerRole;
use crate::producers::{ToolInventory, get_all_tool_descriptions, resolve_worker_tools};

use super::budget::LoopBudget;
use super::error::CoordinatorLoopError;

/// A tool a worker can access, with its description when the Full visibility
/// mode renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerTool {
    name: String,
    description: Option<String>,
}

impl WorkerTool {
    /// The tool name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The tool description, when the Full visibility mode provides it.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// One worker's renderable specification for the roster section.
///
/// Carries everything the roster rendering reads for one worker: the role
/// name, the description, and the resolved tool list. The
/// [`WorkerRoster`] holds a `Vec<WorkerSpec>` so
/// [`WorkerSections::from_roster`](super::driver::WorkerSections::from_roster)
/// can render the roster section from the typed roster alone.
///
/// Forbidden invalid state: a worker spec without a valid role; the
/// constructor delegates to [`WorkerRole`]. A configured turn depth of zero
/// is rejected at construction too, so a spec's budget is a depth the worker
/// can actually spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSpec {
    role: WorkerRole,
    description: String,
    tools: Vec<WorkerTool>,
    preamble: String,
    budget: Option<LoopBudget>,
}

impl WorkerSpec {
    /// The worker's role name.
    pub fn role(&self) -> &WorkerRole {
        &self.role
    }

    /// The worker's description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The tools this worker can access, with descriptions when available.
    pub fn tools(&self) -> &[WorkerTool] {
        &self.tools
    }

    /// The worker's system-prompt preamble, carried from `WorkerConfig` so
    /// the executor can read a worker's system prompt from the typed roster
    /// (R4).
    pub fn preamble(&self) -> &str {
        &self.preamble
    }

    /// The worker's own turn-depth budget, when its configuration set one.
    ///
    /// Carried from `WorkerConfig::turn_depth` for the same reason as
    /// [`preamble`](Self::preamble): the executor resolves a worker's depth
    /// per task from the typed roster. `None` means the configuration named
    /// no depth for this worker, and the executor falls back to the
    /// run-wide default.
    pub fn budget(&self) -> Option<LoopBudget> {
        self.budget
    }
}

/// The workers this run has configured, in configuration order, with the
/// renderable per-worker spec and the tool-visibility inputs the roster
/// section renders.
///
/// The roster is what makes worker assignment checkable: the planning schema
/// offers exactly these names and the plan parse rejects anything else, so a
/// plan can never be dispatched to a worker that does not exist. An empty
/// roster is a run with no workers at all, where naming any worker is the
/// error.
///
/// Beyond names, the roster carries each worker's description and resolved
/// tool list, plus the [`ToolVisibility`] mode and [`ToolListLimit`] that
/// decide how the roster section renders. This is the widening that makes
/// [`WorkerSections::from_roster`](super::driver::WorkerSections::from_roster)
/// implementable from the typed roster alone, removing the parallel-derivation
/// smell where the roster text and the typed roster were produced
/// independently from the same config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRoster {
    workers: Vec<WorkerSpec>,
    tool_visibility: ToolVisibility,
    tool_list_limit: ToolListLimit,
}

impl Default for WorkerRoster {
    fn default() -> Self {
        Self {
            workers: Vec::new(),
            tool_visibility: ToolVisibility::default(),
            tool_list_limit: ToolListLimit::new(0),
        }
    }
}

impl WorkerRoster {
    /// Read the roster from an orchestration configuration, capturing the
    /// renderable per-worker spec and the tool-visibility inputs.
    ///
    /// This is the parse step that captures everything the roster section
    /// renders: per-worker role, description, and resolved tool list (with
    /// descriptions for the Full visibility path), plus the visibility mode
    /// and tool-list limit. It also captures each worker's own turn depth,
    /// which the executor resolves per task.
    ///
    /// Each worker's `mcp_filter` selects from `inventory`, so what the
    /// roster advertises is what the runtime can actually execute. Pass
    /// [`ToolInventory::empty`] for a runtime with no MCP backend.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::ZeroTurnBudget`] when a worker's
    /// configured `turn_depth` is zero, which would build a worker that can
    /// take no turn at all.
    pub fn from_config(
        config: &OrchestrationConfig,
        tool_list_limit: ToolListLimit,
        vector_stores: &[VectorStoreConfig],
        inventory: &ToolInventory,
    ) -> Result<Self, CoordinatorLoopError> {
        let worker_tools = resolve_worker_tools(config, inventory);
        let tool_descriptions = get_all_tool_descriptions(vector_stores);

        let workers: Vec<WorkerSpec> = config
            .workers
            .iter()
            .filter_map(|(name, wc)| WorkerRole::new(name).ok().map(|role| (name, role, wc)))
            .map(|(name, role, wc)| {
                let tools: Vec<WorkerTool> = worker_tools
                    .get(name)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|tool_name| WorkerTool {
                        description: tool_descriptions.get(&tool_name).cloned(),
                        name: tool_name,
                    })
                    .collect();
                let budget = wc
                    .turn_depth
                    .map(|depth| LoopBudget::new(u32::try_from(depth).unwrap_or(u32::MAX)))
                    .transpose()?;
                Ok(WorkerSpec {
                    role,
                    description: wc.description.clone(),
                    tools,
                    preamble: wc.preamble.clone(),
                    budget,
                })
            })
            .collect::<Result<_, CoordinatorLoopError>>()?;

        Ok(Self {
            workers,
            tool_visibility: config.tools_in_planning,
            tool_list_limit,
        })
    }

    /// A run with no workers configured.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The configured role names, in configuration order.
    pub fn names(&self) -> Vec<&str> {
        self.workers.iter().map(|spec| spec.role.as_str()).collect()
    }

    /// The per-worker specs, in configuration order.
    pub fn workers(&self) -> &[WorkerSpec] {
        &self.workers
    }

    /// The tool visibility mode the roster section renders under.
    pub fn tool_visibility(&self) -> ToolVisibility {
        self.tool_visibility
    }

    /// The tool list limit for truncating per-worker tool lists.
    pub fn tool_list_limit(&self) -> ToolListLimit {
        self.tool_list_limit
    }

    /// Whether the roster is empty.
    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }

    /// Check a worker name a plan proposed.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::UnknownWorker`] when no configured
    /// worker carries the name.
    pub fn check(&self, name: &str) -> Result<(), CoordinatorLoopError> {
        if self.workers.iter().any(|spec| spec.role.as_str() == name) {
            return Ok(());
        }
        Err(CoordinatorLoopError::UnknownWorker {
            name: name.to_owned(),
            available: self.listed(),
        })
    }

    /// The configured names as a comma-separated list, or `none` when no
    /// workers are configured.
    pub fn listed(&self) -> String {
        if self.workers.is_empty() {
            return "none".to_owned();
        }
        self.workers
            .iter()
            .map(|spec| spec.role.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
