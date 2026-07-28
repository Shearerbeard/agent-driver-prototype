//! Live sidecar probe for card S72.
//!
//! Connects to a classic-SSE MCP sidecar, runs the full JSON-RPC sequence
//! (initialize, tools/list, tools/call keystrokes, tools/call capture-pane),
//! prints every JSON-RPC request and response verbatim to stdout so the board
//! owner can freeze the transcript and diff it against the F3 capture, and
//! asserts the wire shape: exactly `keystrokes` + `capture-pane` advertised,
//! and the captured pane contains the probe token `S72_PROBE_OK_72`.
//!
//! Usage: `sidecar_probe <sse-base-url>` (e.g. `http://localhost:8000/sse`).
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
        eprintln!("usage: {prog} <sse-base-url>");
        eprintln!("example: {prog} http://localhost:8000/sse");
        exit(2);
    }
    if let Err(error) = run(&args[1]).await {
        eprintln!("sidecar_probe failed: {error}");
        exit(1);
    }
}

async fn run(base_url: &str) -> Result<(), String> {
    let url = SidecarUrl::new(base_url).map_err(|e| e.to_string())?;
    let client = SidecarClient::connect(url)
        .await
        .map_err(|e| e.to_string())?;

    // connect captured the `endpoint` event; emit it first so the transcript
    // opens the same way the F3 capture does.
    emit_transcript(&client);
    println!("# message endpoint: {}", client.message_endpoint());

    // --- initialize -------------------------------------------------------
    let info = client.initialize().await.map_err(|e| e.to_string())?;
    emit_step(&client, "initialize");
    println!("# serverInfo: {} {}", info.server_name, info.server_version);

    // --- tools/list -------------------------------------------------------
    let tools = client.list_tools().await.map_err(|e| e.to_string())?;
    emit_step(&client, "tools/list");
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
    emit_step(&client, "tools/call keystrokes");
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
    emit_step(&client, "tools/call capture-pane");
    println!("# capture-pane:\n{}", pane.as_str());

    if !pane.as_str().contains(PROBE_TOKEN) {
        return Err(format!(
            "capture-pane did not contain {PROBE_TOKEN}; pane: {:?}",
            pane.as_str()
        ));
    }

    // Drain any trailing `: ping` comment frames so the live transcript
    // captures the keep-alive wire shape too.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    emit_transcript(&client);

    Ok(())
}

/// Print the most recent JSON-RPC request body, then drain and print all SSE
/// frames that have arrived since the previous call.
fn emit_step(client: &SidecarClient, label: &str) {
    println!(">>> {label}");
    if let Some(req) = client.last_request() {
        println!("{req}");
    }
    emit_transcript(client);
}

fn emit_transcript(client: &SidecarClient) {
    for block in client.drain_transcript() {
        print!("{block}");
    }
}
