//! The shim's TOML config layer, relocated from the server binary.
//!
//! `src/config.rs` does not parse TOML (its `OrchestrationConfig` is a
//! hand-mirror, not `Deserialize`), so the shim parses a minimal subset
//! here and maps it into the crate's `OrchestrationConfig` for the
//! roster/preamble builders. Malformed TOML fails loud. Relocation only
//! (tb/S109): the shapes and validation are exactly what the binary
//! carried; S104's config ruling binds unchanged.

use std::collections::HashMap;

use serde::Deserialize;

use crate::config::{OrchestrationConfig, ToolVisibility, WorkerConfig};
use crate::mcp_client::SidecarUrl;
use crate::sse_shim::ShimError;

/// The parsed adapter-patched TOML, carrying only the fields the loop needs.
pub struct ShimConfig {
    pub agent: AgentSection,
    pub orchestration: OrchestrationSection,
    /// The one MCP server `[mcp.servers.*]` names, when the TOML has any.
    /// `None` means no `[mcp.servers]` block and `--sidecar-url` applies.
    pub mcp_server: Option<ConfiguredMcpServer>,
    /// The crate's `OrchestrationConfig` mirror, assembled from the parsed
    /// sections for the roster/preamble builders.
    pub orchestration_config: OrchestrationConfig,
}

/// The transport a configured MCP server speaks, resolved from the
/// `transport = "…"` field of its `[mcp.servers.*]` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    /// rmcp's streamable HTTP: one endpoint, `POST` requests, optional
    /// GET stream.
    HttpStreamable,
    /// The legacy 2024-11-05 HTTP+SSE protocol: `GET /sse`, the
    /// `event: endpoint` discovery frame, session POSTs to the resolved
    /// message URL. Carried by the crate's custom rmcp transport.
    LegacySse,
}

/// The MCP server the shim wires, resolved and validated from one
/// `[mcp.servers.<name>]` block.
///
/// Both aura transport names resolve: `http_streamable` onto rmcp's
/// transport, `sse` onto the crate's custom legacy transport. Anything
/// else is rejected at parse time rather than failing opaquely at
/// connect time.
#[derive(Debug)]
pub struct ConfiguredMcpServer {
    pub name: String,
    pub transport: McpTransport,
    pub url: SidecarUrl,
    /// Static headers sent on every request — the auth block the mezmo
    /// server expects.
    pub headers: HashMap<String, String>,
}

/// Resolve `[mcp.servers.*]` into the one server the shim can wire.
///
/// Aura's config names two transports, `http_streamable` and `sse`; both
/// are supported here. An empty or absent servers map is `Ok(None)` —
/// the `--sidecar-url` fallback path.
fn resolve_mcp_server(parsed: &ParsedConfig) -> Result<Option<ConfiguredMcpServer>, ShimError> {
    if parsed.mcp.servers.is_empty() {
        return Ok(None);
    }
    if parsed.mcp.servers.len() > 1 {
        // The server map is a HashMap, so the names are sorted to keep the
        // failure reproducible.
        let mut names: Vec<String> = parsed.mcp.servers.keys().cloned().collect();
        names.sort();
        return Err(ShimError::Server(format!(
            "[mcp.servers] declares {} servers ({}); the worker path holds exactly \
             one client, and multi-server routing is out of scope",
            parsed.mcp.servers.len(),
            names.join(", ")
        )));
    }
    let (name, server) = parsed.mcp.servers.iter().next().expect("len checked above");
    let transport = match server.transport.as_deref() {
        Some("http_streamable") => McpTransport::HttpStreamable,
        Some("sse") => McpTransport::LegacySse,
        Some(other) => {
            return Err(ShimError::Server(format!(
                "mcp server '{name}': unknown transport {other:?}; expected \
                 \"http_streamable\" or \"sse\""
            )));
        }
        None => {
            return Err(ShimError::Server(format!(
                "mcp server '{name}': missing transport; expected \"http_streamable\" \
                 or \"sse\""
            )));
        }
    };
    let url_raw = server.url.as_deref().unwrap_or_default();
    let url = SidecarUrl::new(url_raw)
        .map_err(|e| ShimError::Server(format!("mcp server '{name}': invalid url: {e}")))?;
    Ok(Some(ConfiguredMcpServer {
        name: name.clone(),
        transport,
        url,
        headers: server.headers.clone(),
    }))
}

#[derive(Debug, Default, Deserialize)]
pub struct AgentSection {
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub turn_depth: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
pub struct OrchestrationSection {
    #[serde(default = "default_max_planning_cycles")]
    pub max_planning_cycles: usize,
    #[serde(default)]
    pub worker_system_prompt: Option<String>,
    #[serde(default)]
    pub worker: HashMap<String, WorkerSection>,
    #[serde(default = "default_tools_in_planning")]
    pub tools_in_planning: String,
    #[serde(default = "default_max_tools_per_worker")]
    pub max_tools_per_worker: usize,
    #[serde(default)]
    pub artifacts: ArtifactsSection,
}

#[derive(Debug, Default, Deserialize)]
pub struct ArtifactsSection {
    #[serde(default)]
    pub memory_dir: Option<String>,
    #[serde(default)]
    pub result_artifact_threshold: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WorkerSection {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub preamble: String,
    #[serde(default)]
    pub mcp_filter: Vec<String>,
    #[serde(default)]
    pub vector_stores: Vec<String>,
    #[serde(default)]
    pub turn_depth: Option<usize>,
}

pub fn default_max_planning_cycles() -> usize {
    4
}

fn default_tools_in_planning() -> String {
    "summary".to_owned()
}

fn default_max_tools_per_worker() -> usize {
    10
}

/// Parse the config TOML and assemble the crate's `OrchestrationConfig`
/// mirror from the worker roster, visibility, and budget fields the loop
/// needs.
pub fn load_shim_config(path: &std::path::Path) -> Result<ShimConfig, ShimError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ShimError::Server(format!("failed to read config {}: {e}", path.display())))?;
    let parsed: ParsedConfig = toml::from_str(&text).map_err(|e| {
        ShimError::Server(format!("malformed TOML in config {}: {e}", path.display()))
    })?;

    let tool_visibility = parse_tool_visibility(&parsed.orchestration.tools_in_planning);

    // Resolve before `parsed.orchestration` is moved out below; the borrow
    // ends here.
    let mcp_server = resolve_mcp_server(&parsed)?;

    let mut orchestration_config = OrchestrationConfig {
        enabled: true,
        max_planning_cycles: parsed.orchestration.max_planning_cycles,
        worker_system_prompt: parsed.orchestration.worker_system_prompt.clone(),
        tools_in_planning: tool_visibility,
        max_tools_per_worker: parsed.orchestration.max_tools_per_worker,
        ..Default::default()
    };
    let mut orchestration = parsed.orchestration;
    // Drain the worker map in place so `orchestration` stays whole and can
    // travel into `ShimConfig` for the budget/threshold fields build_state
    // reads later.
    for (name, worker) in orchestration.worker.drain() {
        orchestration_config.workers.insert(
            name,
            WorkerConfig {
                description: worker.description,
                preamble: worker.preamble,
                mcp_filter: worker.mcp_filter,
                vector_stores: worker.vector_stores,
                turn_depth: worker.turn_depth,
                llm: None,
                scratchpad: None,
                skills: None,
            },
        );
    }
    orchestration_config.artifacts.memory_dir = orchestration.artifacts.memory_dir.clone();

    Ok(ShimConfig {
        agent: parsed.agent,
        orchestration,
        mcp_server,
        orchestration_config,
    })
}

/// The raw deserialized TOML (before assembling the crate mirror).
#[derive(Debug, Default, Deserialize)]
struct ParsedConfig {
    #[serde(default)]
    agent: AgentSection,
    #[serde(default)]
    orchestration: OrchestrationSection,
    #[serde(default)]
    mcp: McpSection,
}

/// The `[mcp]` table: the server map plus fields the shim does not
/// implement (`sanitize_schemas` and friends), which serde ignores.
#[derive(Debug, Default, Deserialize)]
struct McpSection {
    #[serde(default)]
    servers: HashMap<String, McpServerSection>,
}

/// One `[mcp.servers.<name>]` block. Unknown fields (`description`,
/// `scratchpad`) are ignored.
#[derive(Debug, Default, Deserialize)]
struct McpServerSection {
    transport: Option<String>,
    url: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
}

/// Map the `tools_in_planning` string to the crate's `ToolVisibility` enum.
fn parse_tool_visibility(raw: &str) -> ToolVisibility {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => ToolVisibility::None,
        "full" => ToolVisibility::Full,
        _ => ToolVisibility::Summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a TOML string the way `load_shim_config` does, so the
    /// `[mcp.servers]` tests exercise the serde shapes of the real config
    /// class rather than hand-built structs.
    fn parse_config(text: &str) -> Result<ParsedConfig, ShimError> {
        toml::from_str(text).map_err(|e| ShimError::Server(format!("malformed TOML: {e}")))
    }

    /// The mezmo-orchestrated shape: one `http_streamable` server with a
    /// static auth header, carrying sections the shim does not implement.
    #[test]
    fn a_mezmo_shaped_servers_block_resolves_to_one_streamable_server_with_headers() {
        let parsed = parse_config(
            r#"
[mcp]
sanitize_schemas = false

[mcp.servers.mezmo_mcp]
transport = "http_streamable"
url = "https://mcp.use.dev.mezmo.it/mcp"
description = "Mezmo MCP server"

[mcp.servers.mezmo_mcp.headers]
Authorization = "Bearer {{ env.MEZMO_API_KEY }}"

[mcp.servers.mezmo_mcp.scratchpad."*"]
min_tokens = 10000
"#,
        )
        .expect("the mezmo fixture shape parses");

        let server = resolve_mcp_server(&parsed)
            .expect("one http_streamable server resolves")
            .expect("a server is configured");
        assert_eq!(server.name, "mezmo_mcp");
        assert_eq!(server.transport, McpTransport::HttpStreamable);
        assert_eq!(server.url.as_str(), "https://mcp.use.dev.mezmo.it/mcp");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Bearer {{ env.MEZMO_API_KEY }}")
        );
    }

    /// The inline-headers form (`headers = { … }`) parses to the same map
    /// as the sub-table form.
    #[test]
    fn inline_headers_parse_like_the_subtable_form() {
        let parsed = parse_config(
            r#"
[mcp.servers.logdnactl]
transport = "http_streamable"
url = "http://mcp-logdnactl:5000/mcp"
headers = { Authorization = "Token token=x" }
"#,
        )
        .expect("inline headers parse");

        let server = resolve_mcp_server(&parsed)
            .expect("resolves")
            .expect("configured");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Token token=x")
        );
    }

    #[test]
    fn no_servers_block_means_none_and_the_cli_url_applies() {
        let parsed = parse_config("[agent]\nname = \"x\"\n").expect("parses");
        assert!(
            resolve_mcp_server(&parsed)
                .expect("empty map is not an error")
                .is_none()
        );

        let parsed = parse_config("").expect("empty file parses");
        assert!(
            resolve_mcp_server(&parsed)
                .expect("absent section is not an error")
                .is_none()
        );
    }

    /// `sse` is aura's other transport name: it resolves onto the legacy
    /// client, keeping the sidecar-style `/sse` server configs the shim
    /// used to reject with the upstream-removal fact.
    #[test]
    fn the_sse_transport_resolves_to_the_legacy_client() {
        let parsed = parse_config(
            r#"
[mcp.servers.terminal]
transport = "sse"
url = "http://localhost:8000/sse"

[mcp.servers.terminal.headers]
Authorization = "Token token=x"
"#,
        )
        .expect("parses");

        let server = resolve_mcp_server(&parsed)
            .expect("sse resolves onto the legacy transport")
            .expect("a server is configured");
        assert_eq!(server.name, "terminal");
        assert_eq!(server.transport, McpTransport::LegacySse);
        assert_eq!(server.url.as_str(), "http://localhost:8000/sse");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Token token=x")
        );
    }

    #[test]
    fn an_unknown_transport_names_the_server() {
        let parsed = parse_config(
            r#"
[mcp.servers.terminal]
transport = "websocket"
url = "http://localhost:9"
"#,
        )
        .expect("parses");

        let error = resolve_mcp_server(&parsed).expect_err("websocket is not a transport");
        assert!(
            error.to_string().contains("'terminal'"),
            "the error names the server, got: {error}"
        );
    }

    #[test]
    fn a_missing_transport_or_url_is_rejected() {
        let parsed = parse_config("[mcp.servers.terminal]\nurl = \"http://localhost:9/mcp\"\n")
            .expect("parses");
        assert!(resolve_mcp_server(&parsed).is_err(), "missing transport");

        let parsed = parse_config("[mcp.servers.terminal]\ntransport = \"http_streamable\"\n")
            .expect("parses");
        assert!(resolve_mcp_server(&parsed).is_err(), "missing url");
    }

    /// The worker path holds one client; the multi-server config class is
    /// out of scope and must fail loud rather than silently drop servers.
    #[test]
    fn two_servers_are_rejected_naming_both() {
        let parsed = parse_config(
            r#"
[mcp.servers.a]
transport = "http_streamable"
url = "http://a:1/mcp"

[mcp.servers.b]
transport = "http_streamable"
url = "http://b:2/mcp"
"#,
        )
        .expect("parses");

        let error = resolve_mcp_server(&parsed).expect_err("one client only");
        let message = error.to_string();
        assert!(
            message.contains('2') && message.contains("a, b"),
            "the error names the count and both servers, got: {message}"
        );
    }
}
