//! Live MCP server probe (originally card S72's sidecar probe).
//!
//! Connects to an MCP server, runs the full sequence (initialize,
//! tools/list, tools/call keystrokes, tools/call capture-pane), and
//! asserts the wire shape the TerminalBench sidecar defines: exactly
//! `keystrokes` + `capture-pane` advertised, and the captured pane
//! contains the probe token `S72_PROBE_OK_72`.
//!
//! Usage: `mcp_probe [--transport sse|http_streamable] <mcp-url>`
//! (e.g. `http://localhost:9992/sse`). The transport defaults to `sse` —
//! the probe's contract is the TB sidecar's `/sse` endpoint and the S72
//! wire assertions; `--transport http_streamable` reaches a
//! streamable-HTTP server. Exits non-zero with the error message on
//! stderr if any step fails.

use std::collections::HashMap;
use std::process::exit;

use agent_driver_prototype::mcp_client::{
    SidecarClient, SidecarToolArgs, SidecarToolName, SidecarUrl,
};
use serde_json::json;

/// `echo S72_PROBE_OK_$((50+22))` evaluates to `S72_PROBE_OK_72` in bash.
/// 50 + 22 = 72, matching this card number.
const PROBE_COMMAND: &str = "echo S72_PROBE_OK_$((50+22))";
const PROBE_TOKEN: &str = "S72_PROBE_OK_72";

/// The transport the probe connects over, taken explicitly from the CLI.
enum ProbeTransport {
    Sse,
    HttpStreamable,
}

impl ProbeTransport {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "sse" => Ok(Self::Sse),
            "http_streamable" => Ok(Self::HttpStreamable),
            other => Err(format!(
                "unknown transport {other:?}; expected \"sse\" or \"http_streamable\""
            )),
        }
    }
}

/// The parsed command line: one `<mcp-url>` plus the transport flag.
struct ProbeArgs {
    url: String,
    transport: ProbeTransport,
}

/// Parse the arguments after the program name.
///
/// `--transport` accepts `sse` or `http_streamable` in both the
/// space-separated and `=`-joined forms and defaults to `sse`. Exactly
/// one non-flag argument, the MCP URL, is required.
fn parse_probe_args(args: &[String]) -> Result<ProbeArgs, String> {
    let mut url = None;
    let mut transport = ProbeTransport::Sse;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--transport" => {
                let value = rest
                    .next()
                    .ok_or_else(|| "--transport needs a value".to_owned())?;
                transport = ProbeTransport::parse(value)?;
            }
            other if other.starts_with("--transport=") => {
                transport = ProbeTransport::parse(&other["--transport=".len()..])?;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other:?}"));
            }
            _ => {
                if url.replace(arg.clone()).is_some() {
                    return Err("expected exactly one <mcp-url>".to_owned());
                }
            }
        }
    }
    let url = url.ok_or_else(|| "a <mcp-url> is required".to_owned())?;
    Ok(ProbeArgs { url, transport })
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let prog = argv.first().map(String::as_str).unwrap_or("mcp_probe");
    let parsed = match parse_probe_args(&argv[1..]) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{prog}: {error}");
            eprintln!("usage: {prog} [--transport sse|http_streamable] <mcp-url>");
            eprintln!("example: {prog} http://localhost:9992/sse");
            exit(2);
        }
    };
    if let Err(error) = run(&parsed.url, parsed.transport).await {
        eprintln!("mcp_probe failed: {error}");
        exit(1);
    }
}

async fn run(base_url: &str, transport: ProbeTransport) -> Result<(), String> {
    let url = SidecarUrl::new(base_url).map_err(|e| e.to_string())?;
    // The transport is explicit; the connect completes the initialize
    // handshake, and initialize() reports it.
    let client = match transport {
        ProbeTransport::Sse => SidecarClient::connect_sse(url, HashMap::new()).await,
        ProbeTransport::HttpStreamable => {
            SidecarClient::connect_streamable(url, HashMap::new()).await
        }
    }
    .map_err(|e| e.to_string())?;

    // --- initialize -------------------------------------------------------
    let info = client.initialize().map_err(|e| e.to_string())?;
    println!(
        "# serverInfo: {} {} (protocol {})",
        info.server_name, info.server_version, info.protocol_version
    );

    // --- tools/list -------------------------------------------------------
    let tools = client.list_tools().await.map_err(|e| e.to_string())?;
    let names: Vec<&str> = tools.iter().map(|t| t.name().as_str()).collect();
    println!("# advertised tools: {names:?}");
    let have_keystrokes = names.contains(&"keystrokes");
    let have_capture_pane = names.contains(&"capture-pane");
    if !(names.len() == 2 && have_keystrokes && have_capture_pane) {
        return Err(format!(
            "tools/list must expose exactly [keystrokes, capture-pane]; got {names:?}"
        ));
    }

    // --- tools/call keystrokes -------------------------------------------
    let ks_name = SidecarToolName::new("keystrokes").map_err(|e| e.to_string())?;
    let ks_args = SidecarToolArgs::from_value(json!({
        "keystrokes": PROBE_COMMAND,
        "append_enter": true,
    }))
    .map_err(|e| e.to_string())?;
    let keystrokes_out = client
        .call_tool(&ks_name, &ks_args)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "# keystrokes result contains token: {}",
        keystrokes_out.as_str().contains(PROBE_TOKEN)
    );

    // --- tools/call capture-pane -----------------------------------------
    let cp_name = SidecarToolName::new("capture-pane").map_err(|e| e.to_string())?;
    let cp_args = SidecarToolArgs::from_value(json!({})).map_err(|e| e.to_string())?;
    let pane = client
        .call_tool(&cp_name, &cp_args)
        .await
        .map_err(|e| e.to_string())?;
    println!("# capture-pane:\n{}", pane.as_str());

    if !pane.as_str().contains(PROBE_TOKEN) {
        return Err(format!(
            "capture-pane did not contain {PROBE_TOKEN}; pane: {:?}",
            pane.as_str()
        ));
    }

    Ok(())
}
