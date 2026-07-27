//! The wrapper that assembles the session, drives one loop, and reads the
//! result back.

use std::sync::Arc;

use agent_driver_rs::agent::{AgentEvent, AgentLoop, AgentLoopConfig, AgentObserver};
use agent_driver_rs::{DynTool, ModelId, Provider, Session, SessionBuilder, SystemPrompt};
use async_trait::async_trait;

use crate::config::ToolVisibility;
use crate::context::PinnedGoal;
use crate::templates::{PlanningLoopVars, render_planning_loop_prompt};

use super::budget::LoopBudget;
use super::error::CoordinatorRunError;
use super::executor::PlanExecutor;
use super::outcome::CoordinatorOutcome;
use super::roster::WorkerRoster;
use super::run_store::RunStore;
use super::terminal::{FinalResponse, TerminalSlot};
use super::tools::{CreatePlanTool, ExecuteTool, InspectRunTool, RespondTool};

/// The worker material one typed [`WorkerRoster`] produces for the loop.
///
/// The roster text, the assignment guidelines and the worker-field fragment
/// are rendered together from one typed roster and travel together, so the
/// planning message cannot describe one roster while the planning schema
/// offers another.
#[derive(Debug, Clone, Default)]
pub struct WorkerSections {
    roster_section: String,
    worker_field: String,
    guidelines: String,
    roster: WorkerRoster,
}

impl WorkerSections {
    /// Render every worker section from a typed [`WorkerRoster`].
    ///
    /// This is the single-derivation path: the roster text, the worker-field
    /// fragment, and the guidelines are all rendered from the typed roster,
    /// so a prose/schema roster mismatch is unrepresentable. The widened
    /// roster carries each worker's role, description, resolved tool list
    /// (with descriptions for the Full visibility path), and the
    /// tool-visibility inputs, so the render reads the roster alone.
    pub fn from_roster(roster: WorkerRoster) -> Self {
        if roster.is_empty() {
            return Self {
                roster_section: String::new(),
                worker_field: String::new(),
                guidelines: String::new(),
                roster,
            };
        }

        let worker_field = r#",
      "worker": "worker_name""#.to_string();

        let names_json: Vec<String> = roster
            .names()
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect();
        let guidelines = crate::templates::render_worker_guidelines(
            &crate::templates::WorkerGuidelinesVars {
                valid_worker_names: &names_json.join(", "),
            },
        );

        let roster_section = render_roster_section(&roster);

        Self {
            roster_section,
            worker_field,
            guidelines,
            roster,
        }
    }

    /// A run with no workers configured, which is what the producer returns
    /// for a configuration that has none.
    pub fn none() -> Self {
        Self::default()
    }

    /// The rendered roster section of the planning message.
    pub fn roster_section(&self) -> &str {
        &self.roster_section
    }

    /// The ported worker-field fragment, which shows the model the exact
    /// shape of an assigned task step.
    pub fn worker_field(&self) -> &str {
        &self.worker_field
    }

    /// The rendered worker-assignment guidelines.
    pub fn guidelines(&self) -> &str {
        &self.guidelines
    }

    /// The names a plan may assign work to.
    pub fn roster(&self) -> &WorkerRoster {
        &self.roster
    }
}

/// Render the roster section from the typed roster, dispatching on the
/// visibility mode the roster carries. Each branch mirrors the
/// `build_workers_section_*` oracle in `producers` but reads the typed
/// [`WorkerRoster`] instead of the raw config, so the parallel derivation
/// is removed.
fn render_roster_section(roster: &WorkerRoster) -> String {
    match roster.tool_visibility() {
        ToolVisibility::None => render_roster_no_tools(roster),
        ToolVisibility::Summary => render_roster_summary_tools(roster),
        ToolVisibility::Full => render_roster_full_tools(roster),
    }
}

fn render_roster_no_tools(roster: &WorkerRoster) -> String {
    let roster_content = roster
        .workers()
        .iter()
        .map(|spec| format!("- {}: {}", spec.role().as_str(), spec.description()))
        .collect::<Vec<_>>()
        .join("\n");
    crate::templates::render_worker_roster(&crate::templates::WorkerRosterVars {
        header_note: "",
        roster_content: &roster_content,
        closing_line: "Each worker has specialized capabilities. Assign tasks to the most appropriate worker.",
    })
}

fn render_roster_summary_tools(roster: &WorkerRoster) -> String {
    let max_tools = roster.tool_list_limit().get();
    let sections: Vec<String> = roster
        .workers()
        .iter()
        .map(|spec| {
            let tools: Vec<String> = spec.tools().iter().map(|t| t.name().to_owned()).collect();
            let tool_list = crate::producers::format_tool_list(&tools, max_tools);
            if tool_list.is_empty() {
                format!(
                    "## {}\n{}\nTools: (none configured — this worker cannot query external systems)",
                    spec.role().as_str(),
                    spec.description()
                )
            } else {
                format!(
                    "## {}\n{}\nTools: {}",
                    spec.role().as_str(),
                    spec.description(),
                    tool_list
                )
            }
        })
        .collect();
    crate::templates::render_worker_roster(&crate::templates::WorkerRosterVars {
        header_note: "NOTE: Worker names below are role assignments, not callable tool names. Only the tools listed under each worker are MCP tools that workers can execute.\n\n",
        roster_content: &sections.join("\n\n"),
        closing_line: "Assign tasks to the worker whose tools best match the required operations.",
    })
}

fn render_roster_full_tools(roster: &WorkerRoster) -> String {
    let max_tools = roster.tool_list_limit().get();
    let sections: Vec<String> = roster
        .workers()
        .iter()
        .map(|spec| {
            let tool_details: Vec<String> = spec
                .tools()
                .iter()
                .take(max_tools)
                .map(|t| match t.description() {
                    Some(desc) => format!("  - {}: {}", t.name(), desc),
                    None => format!("  - {}", t.name()),
                })
                .collect();
            let remaining = spec.tools().len().saturating_sub(max_tools);
            let tool_section = if tool_details.is_empty() {
                String::new()
            } else if remaining > 0 {
                format!("{}\n  (+{} more)", tool_details.join("\n"), remaining)
            } else {
                tool_details.join("\n")
            };
            if tool_section.is_empty() {
                format!("## {}\n{}", spec.role().as_str(), spec.description())
            } else {
                format!(
                    "## {}\n{}\nTools:\n{}",
                    spec.role().as_str(),
                    spec.description(),
                    tool_section
                )
            }
        })
        .collect();
    crate::templates::render_worker_roster(&crate::templates::WorkerRosterVars {
        header_note: "NOTE: Worker names below are role assignments, not callable tool names. Only the tools listed under each worker are MCP tools that workers can execute.\n\n",
        roster_content: &sections.join("\n\n"),
        closing_line: "Assign tasks to the worker whose tools best match the required operations.",
    })
}

/// Everything the loop needs before its first provider call.
///
/// The system prompt is supplied rather than composed here: the ported
/// preamble builder describes the bounded router's tool surface, which is
/// not the surface this loop registers, so composing it in would ship a
/// system prompt that contradicts the tools.
///
/// The run store is supplied rather than created internally so a real
/// executor (the [`DagExecutor`](crate::dag_executor::DagExecutor)) can
/// share the same store the coordinator's tools read and write: the
/// executor reads `latest_plan()` to resolve the plan id and files
/// per-task records that `inspect_run` reads back. A caller creates one
/// [`RunStore`], hands it to both the executor and this config, and the
/// loop's `Arc`-shared handle keeps them joined.
pub struct CoordinatorLoopConfig {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    pub system_prompt: SystemPrompt,
    pub budget: LoopBudget,
    pub executor: Arc<dyn PlanExecutor>,
    pub worker_sections: WorkerSections,
    pub runs: RunStore,
}

/// Forwards loop events to a shared observer handle.
///
/// The substrate takes an owned observer, so a caller that wants to read the
/// events after the run hands in a handle and keeps a clone.
struct SharedObserver(Arc<dyn AgentObserver>);

#[async_trait]
impl AgentObserver for SharedObserver {
    async fn on_event(&self, event: &AgentEvent) {
        self.0.on_event(event).await;
    }
}

/// One coordinator run: one session, one budget, one answer.
///
/// Running consumes the loop. The answer slot and the run records belong to
/// a single run, so a second run over the same loop would inherit an answer
/// it did not write; making `run` take ownership removes that state rather
/// than documenting it. Both are handles, so a caller clones what it wants
/// to read before handing the loop over.
pub struct CoordinatorLoop {
    session: Session,
    budget: LoopBudget,
    answer: TerminalSlot<FinalResponse>,
    runs: RunStore,
    worker_sections: WorkerSections,
    observer: Option<Arc<dyn AgentObserver>>,
}

impl CoordinatorLoop {
    /// Build the session, register the loop tools, and arm the budget.
    ///
    /// The registered surface is `create_plan`, `execute`, `inspect_run` and
    /// `respond`. Worker result submission is deliberately absent: it is a
    /// worker's tool, and mounting it here would offer the coordinator a way
    /// to report evidence it never gathered.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorRunError::Session`] when the provider and model
    /// do not yield a session.
    pub async fn new(config: CoordinatorLoopConfig) -> Result<Self, CoordinatorRunError> {
        let runs = config.runs.clone();
        let answer: TerminalSlot<FinalResponse> = TerminalSlot::new();

        let create_plan: DynTool =
            Arc::new(CreatePlanTool::new(runs.clone(), &config.worker_sections));
        let execute: DynTool =
            Arc::new(ExecuteTool::new(runs.clone(), Arc::clone(&config.executor)));
        let inspect_run: DynTool = Arc::new(InspectRunTool::new(runs.clone()));
        let respond: DynTool = Arc::new(RespondTool::new(answer.clone()));

        let session = SessionBuilder::new()
            .provider(config.provider)
            .model(config.model)
            .system_prompt(config.system_prompt)
            .tools([create_plan, execute, inspect_run, respond])
            .build()
            .await?;

        Ok(Self {
            session,
            budget: config.budget,
            answer,
            runs,
            worker_sections: config.worker_sections,
            observer: None,
        })
    }

    /// Watch the loop's events as they happen.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn AgentObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// The run's records, shareable before the run consumes the loop.
    ///
    /// [`RunStore`] is a handle, so a caller that clones it here still sees
    /// what the loop wrote once the run is over.
    pub fn runs(&self) -> &RunStore {
        &self.runs
    }

    /// The run's answer slot, shareable before the run consumes the loop.
    ///
    /// Symmetric with [`runs`](Self::runs), and the only way to recover a
    /// committed answer from a run that ends in
    /// [`CoordinatorRunError`](super::CoordinatorRunError) rather than an
    /// outcome.
    pub fn answer(&self) -> &TerminalSlot<FinalResponse> {
        &self.answer
    }

    /// Run the loop over one user query.
    ///
    /// The opening message is the rendered loop-shaped planning wrapper,
    /// which names the four tools this loop registers (`create_plan`,
    /// `execute`, `inspect_run`, `respond`) rather than the bounded
    /// router's three. Everything after it is ordinary conversation
    /// history: tool calls and their observations, with no state replayed
    /// into a prompt.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorRunError::AgentLoop`] when the substrate loop
    /// fails outright. A loop that stops for any reported reason, the turn
    /// budget included, is an outcome rather than an error.
    pub async fn run(self, query: &PinnedGoal) -> Result<CoordinatorOutcome, CoordinatorRunError> {
        let timestamp =
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let message = render_planning_loop_prompt(&PlanningLoopVars {
            timestamp: &timestamp,
            query: query.as_str(),
            worker_section: self.worker_sections.roster_section(),
            worker_guidelines: self.worker_sections.guidelines(),
        });

        let config = AgentLoopConfig {
            max_tool_depth: self.budget.into(),
            ..AgentLoopConfig::default()
        };

        let mut agent = AgentLoop::new(&self.session).with_config(config);
        if let Some(observer) = &self.observer {
            agent = agent.with_observer(SharedObserver(Arc::clone(observer)));
        }

        let outcome = agent.run(message).await?;
        Ok(CoordinatorOutcome::interpret(
            outcome,
            &self.answer,
            &self.runs,
        ))
    }
}
