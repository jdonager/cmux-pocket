//! Read-only WebSocket probe against running Gateway instance.

use crate::error::CliError;
use futures_util::{SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// Default timeout for probe network interactions.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// Structured results from probing a running cmux-pocket Gateway.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeReport {
    pub connected: bool,
    pub authenticated: bool,
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_status: Option<Value>,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_health: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProbeReport {
    pub fn is_backend_healthy(&self) -> bool {
        self.authenticated
            && self.backend_health.as_deref() == Some("healthy")
            && self.error.is_none()
    }
}

async fn receive_rpc_result<S>(
    read: &mut S,
    request_id: &str,
    timeout_duration: Duration,
) -> Result<Value, CliError>
where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    timeout(timeout_duration, async {
        while let Some(message) = read.next().await {
            let message = message.map_err(|e| {
                CliError::RuntimeFailure(format!("Read error during Gateway probe: {e}"))
            })?;
            match message {
                Message::Text(text) => {
                    let response: Value = serde_json::from_str(&text).map_err(|e| {
                        CliError::RuntimeFailure(format!("Invalid RPC response JSON: {e}"))
                    })?;
                    if response.get("id").and_then(Value::as_str) != Some(request_id) {
                        continue;
                    }
                    if let Some(error) = response.get("error") {
                        let message = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("RPC error");
                        return Err(CliError::DependencyUnavailable(format!(
                            "Gateway returned RPC error: {message}"
                        )));
                    }
                    return response.get("result").cloned().ok_or_else(|| {
                        CliError::RuntimeFailure("Gateway RPC response has no result".to_string())
                    });
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        Err(CliError::DependencyUnavailable(
            "Gateway disconnected during probe".to_string(),
        ))
    })
    .await
    .map_err(|_| {
        CliError::DependencyUnavailable(format!(
            "Timed out waiting for Gateway response {request_id}"
        ))
    })?
}

/// Connects to a running Gateway over loopback WebSocket, performs authentication,
/// and checks host status and workspace access from the Gateway's own process.
pub async fn probe_gateway(
    host: &str,
    port: u16,
    token: &str,
    timeout_duration: Duration,
) -> Result<ProbeReport, CliError> {
    let ws_url = format!("ws://{}:{}/", host, port);
    let start_time = Instant::now();

    let connect_fut = connect_async(&ws_url);
    let (ws_stream, _) = match timeout(timeout_duration, connect_fut).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(CliError::DependencyUnavailable(format!(
                "Failed to connect to Gateway at {}: {}",
                ws_url, e
            )));
        }
        Err(_) => {
            return Err(CliError::DependencyUnavailable(format!(
                "Timed out connecting to Gateway at {}",
                ws_url
            )));
        }
    };

    let (mut write, mut read) = ws_stream.split();

    // 1. Send auth frame
    let auth_msg = json!({
        "type": "auth",
        "token": token,
        "client_id": "cmux-pocket-cli-probe",
    });

    if let Err(e) = write.send(Message::Text(auth_msg.to_string())).await {
        return Err(CliError::RuntimeFailure(format!(
            "Failed to send auth frame to Gateway: {}",
            e
        )));
    }

    // 2. Receive auth response
    let auth_res = timeout(timeout_duration, read.next()).await;
    let (session_id, server_version, capabilities) = match auth_res {
        Ok(Some(Ok(msg))) => match msg {
            Message::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).map_err(|e| {
                    CliError::RuntimeFailure(format!("Invalid auth response JSON: {}", e))
                })?;

                let msg_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if msg_type == "auth_ok" {
                    let sid = parsed
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let s_ver = parsed
                        .get("server_version")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let caps = parsed
                        .get("capabilities")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|c| c.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    (sid, s_ver, caps)
                } else if msg_type == "auth_error" {
                    let reason = parsed
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("authentication rejected");
                    return Err(CliError::ConfigOrToken(format!(
                        "Gateway authentication rejected: {}",
                        reason
                    )));
                } else {
                    return Err(CliError::RuntimeFailure(format!(
                        "Unexpected response during auth: {}",
                        text
                    )));
                }
            }
            Message::Close(frame) => {
                let code = frame.as_ref().map(|f| f.code.into()).unwrap_or(0);
                if code == 1008 {
                    return Err(CliError::ConfigOrToken(
                        "Gateway closed connection with code 1008 (authentication failed)"
                            .to_string(),
                    ));
                }
                return Err(CliError::DependencyUnavailable(format!(
                    "Gateway closed connection during auth: {:?}",
                    frame
                )));
            }
            _ => {
                return Err(CliError::RuntimeFailure(
                    "Unexpected binary/non-text frame from Gateway during auth".to_string(),
                ));
            }
        },
        Ok(Some(Err(e))) => {
            return Err(CliError::RuntimeFailure(format!(
                "Read error during auth: {}",
                e
            )));
        }
        Ok(None) => {
            return Err(CliError::DependencyUnavailable(
                "Gateway closed connection immediately".to_string(),
            ));
        }
        Err(_) => {
            return Err(CliError::DependencyUnavailable(
                "Timed out waiting for auth response from Gateway".to_string(),
            ));
        }
    };

    // 3. Send read-only mobile.host.status RPC
    let rpc_req = json!({
        "id": "probe-status-1",
        "method": "mobile.host.status",
        "params": {},
    });

    if let Err(e) = write.send(Message::Text(rpc_req.to_string())).await {
        return Err(CliError::RuntimeFailure(format!(
            "Failed to send RPC request: {}",
            e
        )));
    }

    let host_status = receive_rpc_result(&mut read, "probe-status-1", timeout_duration).await?;
    let mut backend_health = host_status
        .get("backend_health")
        .and_then(|health| health.get("status"))
        .and_then(Value::as_str)
        .map(String::from);

    // A probe inside a cmux terminal can ping successfully while the launchd
    // Gateway lacks socket access. Query workspaces through the Gateway itself.
    let workspace_req =
        json!({"id": "probe-workspaces-1", "method": "mobile.workspace.list", "params": {}});
    let workspace_result = async {
        write
            .send(Message::Text(workspace_req.to_string()))
            .await
            .map_err(|e| {
                CliError::RuntimeFailure(format!("Failed to send workspace probe: {e}"))
            })?;
        let result = receive_rpc_result(&mut read, "probe-workspaces-1", timeout_duration).await?;
        if !result.get("workspaces").is_some_and(Value::is_array) {
            return Err(CliError::RuntimeFailure(
                "Gateway workspace response has no workspace list".to_string(),
            ));
        }
        // Workspace names and contents are deliberately not retained in the report.
        Ok(())
    }
    .await;

    let error = match workspace_result {
        Ok(()) => {
            backend_health.get_or_insert_with(|| "healthy".to_string());
            None
        }
        Err(error) => {
            backend_health = Some("unhealthy".to_string());
            Some(format!("Gateway workspace query failed: {error}. Check cmux socket access: 'cmux processes only' rejects launchd; run the Gateway in a cmux terminal or configure authenticated local automation."))
        }
    };

    // Send clean close
    let _ = write.send(Message::Close(None)).await;

    let elapsed = start_time.elapsed().as_millis() as u64;

    Ok(ProbeReport {
        connected: true,
        authenticated: true,
        host: host.to_string(),
        port,
        server_version,
        session_id,
        capabilities,
        host_status: Some(host_status),
        latency_ms: elapsed,
        backend_health,
        error,
    })
}
