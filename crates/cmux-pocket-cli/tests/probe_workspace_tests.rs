use cmux_pocket_cli::cli::DoctorArgs;
use cmux_pocket_cli::commands::handle_doctor;
use cmux_pocket_cli::probe_gateway;
use cmux_pocket_macos::{
    create_dir_user_only, save_config, save_token, GatewayConfig, PocketPaths,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::{accept_async, tungstenite::Message};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn denied_workspaces() -> Value {
    json!({"error": {"code": -32000, "message": "Backend unavailable: cmux tree failed with exit code 1"}})
}

async fn spawn_gateway(
    workspace_response: Value,
    older_gateway: bool,
    unrelated: bool,
) -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut ws = loop {
            let (stream, _) = listener.accept().await.unwrap();
            // status first opens a plain TCP connection to check the listener.
            if let Ok(ws) = accept_async(stream).await {
                break ws;
            }
        };
        while let Some(Ok(message)) = ws.next().await {
            match message {
                Message::Text(text) => {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    let mut response = if request["type"] == "auth" {
                        assert_eq!(request["token"], TOKEN);
                        json!({"type": "auth_ok", "server_version": "2.0.0", "capabilities": []})
                    } else {
                        match request["method"].as_str().unwrap() {
                            "mobile.host.status" => {
                                // Authentication and a healthy host response do not prove tree access.
                                let result = if older_gateway {
                                    json!({})
                                } else {
                                    json!({"backend_health": {"status": "healthy"}})
                                };
                                json!({"id": request["id"], "result": result})
                            }
                            "mobile.workspace.list" => {
                                let mut response = workspace_response.clone();
                                response["id"] = request["id"].clone();
                                response
                            }
                            method => panic!("unexpected RPC method: {method}"),
                        }
                    };
                    if unrelated && request.get("method").is_some() {
                        ws.send(Message::Text(
                            json!({"event": "workspace.tree", "data": {}}).to_string(),
                        ))
                        .await
                        .unwrap();
                        ws.send(Message::Text(
                            json!({"id": "unrelated", "result": {"workspaces": []}}).to_string(),
                        ))
                        .await
                        .unwrap();
                    }
                    ws.send(Message::Text(response.take().to_string()))
                        .await
                        .unwrap();
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });
    (port, task)
}

fn configured_home(temp: &TempDir, port: u16) -> PocketPaths {
    let paths = PocketPaths::from_home_dir(temp.path());
    create_dir_user_only(&paths.config_dir).unwrap();
    let config = GatewayConfig {
        port,
        token_path: Some(paths.token_file.clone()),
        // Simulate a successful CLI ping independently of the gateway process's tree access.
        cmux_path: Some("/usr/bin/true".into()),
        ..GatewayConfig::default()
    };
    save_config(&paths.config_file, &config).unwrap();
    save_token(&paths.token_file, TOKEN).unwrap();
    paths
}

#[tokio::test]
async fn probe_detects_workspace_denial_despite_healthy_host_status() {
    let (port, task) = spawn_gateway(denied_workspaces(), false, false).await;
    let report = probe_gateway("127.0.0.1", port, TOKEN, Duration::from_secs(2))
        .await
        .unwrap();
    assert!(report.authenticated);
    assert_eq!(report.backend_health.as_deref(), Some("unhealthy"));
    assert!(report.error.unwrap().contains("workspace"));
    task.await.unwrap();
}

#[tokio::test]
async fn probe_accepts_empty_workspace_lists_from_older_gateways() {
    let (port, task) = spawn_gateway(json!({"result": {"workspaces": []}}), true, false).await;
    let report = probe_gateway("127.0.0.1", port, TOKEN, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(report.backend_health.as_deref(), Some("healthy"));
    assert!(report.error.is_none());
    task.await.unwrap();
}

#[tokio::test]
async fn probe_rejects_a_success_response_without_a_workspace_list() {
    let (port, task) = spawn_gateway(json!({"result": {}}), false, false).await;
    let report = probe_gateway("127.0.0.1", port, TOKEN, Duration::from_secs(2))
        .await
        .unwrap();
    assert!(!report.is_backend_healthy());
    assert!(report.error.unwrap().contains("no workspace list"));
    task.await.unwrap();
}

#[tokio::test]
async fn probe_matches_response_ids_and_keeps_workspace_contents_out_of_reports() {
    let (port, task) = spawn_gateway(
        json!({"result": {"workspaces": [{"name": "private-workspace-name"}]}}),
        true,
        true,
    )
    .await;
    let report = probe_gateway("127.0.0.1", port, TOKEN, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(report.backend_health.as_deref(), Some("healthy"));
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains("private-workspace-name"));
    task.await.unwrap();
}

#[tokio::test]
async fn status_does_not_report_healthy_when_gateway_workspace_access_fails() {
    let (port, task) = spawn_gateway(denied_workspaces(), false, false).await;
    let temp = TempDir::new().unwrap();
    let paths = configured_home(&temp, port);
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_cmux-pocket"))
        .args([
            "--config",
            paths.config_file.to_str().unwrap(),
            "--json",
            "status",
        ])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &envelope["data"];
    assert_eq!(data["cmux"]["ping_ok"], true);
    assert_eq!(data["gateway"]["authenticated_probe"], true);
    assert_eq!(data["gateway"]["backend_health"], "unhealthy");
    assert!(data["overall_status"]
        .as_str()
        .unwrap()
        .starts_with("degraded"));
    task.await.unwrap();
}

#[tokio::test]
async fn doctor_fails_when_gateway_cannot_query_workspaces() {
    let (port, task) = spawn_gateway(denied_workspaces(), false, false).await;
    let temp = TempDir::new().unwrap();
    let paths = configured_home(&temp, port);
    let result = handle_doctor(
        &paths,
        &DoctorArgs {
            offline: false,
            deep: true,
        },
        true,
    )
    .await;
    assert!(result.is_err());
    task.await.unwrap();
}

#[tokio::test]
async fn gateway_probe_exits_with_dependency_error_when_workspace_access_fails() {
    let (port, task) = spawn_gateway(denied_workspaces(), false, false).await;
    let temp = TempDir::new().unwrap();
    let paths = configured_home(&temp, port);
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_cmux-pocket"))
        .args([
            "--config",
            paths.config_file.to_str().unwrap(),
            "--json",
            "gateway",
            "probe",
        ])
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert!(envelope["message"].as_str().unwrap().contains("workspace"));
    task.await.unwrap();
}
