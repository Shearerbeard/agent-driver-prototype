//! Live MCP server probe (originally card S72's sidecar probe).
//!
//! Connects to an MCP server over rmcp streamable HTTP, runs the full
//! sequence (initialize, tools/list, tools/call keystrokes, tools/call
//! capture-pane), and asserts the wire shape the TerminalBench sidecar
//! defines: exactly `keystrokes` + `capture-pane` advertised, and the
//! captured pane contains the probe token `S72_PROBE_OK_72`.
//!
//! Usage: `sidecar_probe <mcp-url>` (e.g. `http://localhost:9992/mcp`).
//! Exits non-zero with the error message on stderr if any step fails.

use std::process::exit;

use agent_driver_prototype::mcp_client::{
    SidecarClient, SidecarToolArgs, SidecarToolName, SidecarUrl,
};
use serde_json::json;

/// `echo S72_PROBE_OK_$((50+22))` evaluates to `S72_PROBE_OK_72` in bash.
/// 50 + 22 = 72, matching this card number.
const PROBE_COMMAND: &str = "echo S72_PROBE_OK_$((50+22))";
const PROBE_TOKEN: &str = "S72_PROBE_OK_72";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        let prog = args.first().map(String::as_str).unwrap_or("sidecar_probe");
        eprintln!("usage: {prog} <mcp-url>");
        eprintln!("example: {prog} http://localhost:9992/mcp");
        exit(2);
    }
    if let Err(error) = run(&args[1]).await {
        eprintln!("sidecar_probe failed: {error}");
        exit(1);
    }
}

async fn run(base_url: &str) -> Result<(), String> {
    let url = SidecarUrl::new(base_url).map_err(|e| e.to_string())?;
    // connect completes the initialize handshake; initialize() reports it.
    let client = SidecarClient::connect(url)
        .await
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
