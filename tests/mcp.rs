use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn mcp_binary_writes_only_jsonrpc_messages_to_stdout() {
    let binary = assert_cmd::cargo::cargo_bin("logit-mcp");
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn logit-mcp");

    {
        let stdin = child.stdin.as_mut().expect("stdin available");
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26"
                }
            })
        )
        .expect("write initialize request");
    }
    child.stdin.take();

    let output = child.wait_with_output().expect("wait for logit-mcp");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();

    assert_eq!(lines.len(), 1, "unexpected stdout: {stdout}");

    let response: Value = serde_json::from_str(lines[0]).expect("stdout line is jsonrpc");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "logit-mcp");
}
