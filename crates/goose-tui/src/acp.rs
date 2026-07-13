//! ACP client for the goose terminal UI.
//!
//! Spawns `goose acp` as a child process and communicates with it over stdio
//! using the Agent Client Protocol. Session updates (agent text, tool calls,
//! tool-call updates) are delivered to the UI as [`AcpEvent`]s over an
//! unbounded channel; permission requests are auto-approved. A separate
//! control channel carries configuration/extension requests (provider/model,
//! extensions) and session (re)creation, again reported back as events.

use agent_client_protocol::schema::v1::{
    ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, TextContent,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{Agent, Client, ConnectionTo};
use goose_sdk_types::custom_requests::{
    AddConfigExtensionRequest, ConfigReadAllRequest, ConfigReadAllResponse, ConfigReadRequest,
    ConfigUpsertRequest, GetAvailableExtensionsRequest, GetAvailableExtensionsResponse,
    GetConfigExtensionsRequest, GetConfigExtensionsResponse, GooseExtension,
    RemoveConfigExtensionRequest, SetConfigExtensionEnabledRequest,
};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// A snapshot of a tool call as reported by the agent.
#[derive(Debug, Clone)]
pub struct ToolCallView {
    pub id: String,
    pub title: String,
    pub status: String,
    pub kind: Option<String>,
    pub raw_input: Option<String>,
    pub raw_output: Option<String>,
}

/// Events streamed from the ACP session to the UI.
#[derive(Debug)]
pub enum AcpEvent {
    /// Session is created and ready to receive prompts.
    Ready { session_id: String },
    /// No provider is configured; the UI should show onboarding.
    NeedOnboarding,
    /// A chunk of streamed agent text.
    AgentChunk(String),
    /// A new tool call was initiated.
    ToolCall(ToolCallView),
    /// An in-flight tool call changed status / output.
    ToolCallUpdate {
        id: String,
        title: Option<String>,
        status: Option<String>,
        raw_input: Option<String>,
        raw_output: Option<String>,
    },
    /// The agent finished responding to a prompt.
    Stopped { stop_reason: String },
    /// A configuration snapshot (provider/model) was read.
    ConfigSnapshot(ConfigReadAllResponse),
    /// The list of configured extensions.
    Extensions(GetConfigExtensionsResponse),
    /// The list of available extensions.
    AvailableExtensions(GetAvailableExtensionsResponse),
    /// Result of a config-save or extension operation.
    OpResult(Result<(), String>),
    /// A protocol or session error occurred.
    Error(String),
}

/// Control messages sent from the UI into the ACP connection loop.
#[derive(Debug)]
pub enum Control {
    NewSession,
    ConfigReadAll,
    ConfigUpsert {
        key: String,
        value: serde_json::Value,
    },
    ListExtensions,
    ListAvailableExtensions,
    AddExtension {
        extension: GooseExtension,
    },
    RemoveExtension {
        config_key: String,
    },
    SetExtensionEnabled {
        config_key: String,
        enabled: bool,
    },
}

/// Handle to the running ACP session.
pub struct AcpClient {
    pub(crate) prompt_tx: mpsc::UnboundedSender<String>,
    pub(crate) control_tx: mpsc::UnboundedSender<Control>,
}

impl AcpClient {
    /// Send a user prompt to the session.
    pub fn send_prompt(&self, text: &str) {
        let _ = self.prompt_tx.send(text.to_string());
    }

    /// (Re)create the session after configuration changes.
    pub fn new_session(&self) {
        let _ = self.control_tx.send(Control::NewSession);
    }

    /// Request a provider/model configuration snapshot.
    pub fn config_read_all(&self) {
        let _ = self.control_tx.send(Control::ConfigReadAll);
    }

    /// Persist a configuration value (e.g. GOOSE_PROVIDER / GOOSE_MODEL).
    pub fn config_upsert(&self, key: &str, value: serde_json::Value) {
        let _ = self.control_tx.send(Control::ConfigUpsert {
            key: key.to_string(),
            value,
        });
    }

    /// Request the list of configured extensions.
    pub fn list_extensions(&self) {
        let _ = self.control_tx.send(Control::ListExtensions);
    }

    /// Request the list of available extensions.
    pub fn list_available_extensions(&self) {
        let _ = self.control_tx.send(Control::ListAvailableExtensions);
    }

    /// Add an extension to the global config.
    pub fn add_extension(&self, extension: GooseExtension) {
        let _ = self.control_tx.send(Control::AddExtension { extension });
    }

    /// Remove a configured extension by its config key.
    pub fn remove_extension(&self, config_key: &str) {
        let _ = self.control_tx.send(Control::RemoveExtension {
            config_key: config_key.to_string(),
        });
    }

    /// Enable or disable a configured extension.
    pub fn set_extension_enabled(&self, config_key: &str, enabled: bool) {
        let _ = self.control_tx.send(Control::SetExtensionEnabled {
            config_key: config_key.to_string(),
            enabled,
        });
    }
}

fn handle_notification(
    notification: SessionNotification,
    tx: &mpsc::UnboundedSender<AcpEvent>,
) -> anyhow::Result<()> {
    match &notification.update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let ContentBlock::Text(text) = &chunk.content {
                tx.send(AcpEvent::AgentChunk(text.text.clone()))
                    .map_err(|_| anyhow::anyhow!("event channel closed"))?;
            }
        }
        SessionUpdate::ToolCall(tc) => {
            tx.send(AcpEvent::ToolCall(ToolCallView {
                id: tc.tool_call_id.to_string(),
                title: tc.title.clone(),
                status: format!("{:?}", tc.status),
                kind: Some(format!("{:?}", tc.kind)),
                raw_input: tc.raw_input.as_ref().map(|v| v.to_string()),
                raw_output: tc.raw_output.as_ref().map(|v| v.to_string()),
            }))
            .map_err(|_| anyhow::anyhow!("event channel closed"))?;
        }
        SessionUpdate::ToolCallUpdate(u) => {
            tx.send(AcpEvent::ToolCallUpdate {
                id: u.tool_call_id.to_string(),
                title: u.fields.title.clone(),
                status: u.fields.status.as_ref().map(|s| format!("{:?}", s)),
                raw_input: u.fields.raw_input.as_ref().map(|v| v.to_string()),
                raw_output: u.fields.raw_output.as_ref().map(|v| v.to_string()),
            })
            .map_err(|_| anyhow::anyhow!("event channel closed"))?;
        }
        _ => {}
    }
    Ok(())
}

/// Connect to a goose ACP agent, spawning `goose_bin acp` as a child process.
///
/// Returns the sending handle plus the receiver that streams [`AcpEvent`]s. The
/// handle and receiver are split so the UI's event loop can own the receiver
/// while the application state holds the sending handle without aliasing the
/// `!Send` connection future.
pub fn connect(goose_bin: PathBuf) -> (AcpClient, mpsc::UnboundedReceiver<AcpEvent>) {
    let (event_tx, events) = mpsc::unbounded_channel::<AcpEvent>();
    let (prompt_tx, mut prompt_rx) = mpsc::unbounded_channel::<String>();
    let (control_tx, mut control_rx) = mpsc::unbounded_channel::<Control>();
    let event_tx_err = event_tx.clone();

    let mut child = tokio::process::Command::new(&goose_bin)
        .arg("acp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn `goose acp`");

    let child_stdin = child.stdin.take().expect("stdin should be piped");
    let child_stdout = child.stdout.take().expect("stdout should be piped");

    let transport =
        agent_client_protocol::ByteStreams::new(child_stdin.compat_write(), child_stdout.compat());

    let connect_fut = Client
        .builder()
        .name("goose-tui")
        .on_receive_notification(
            {
                let event_tx = event_tx.clone();
                move |notification: SessionNotification, _cx| {
                    let event_tx = event_tx.clone();
                    async move {
                        handle_notification(notification, &event_tx).ok();
                        Ok(())
                    }
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                let option_id = request.options.first().map(|o| o.option_id.clone());
                match option_id {
                    Some(id) => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)),
                    )),
                    None => responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    )),
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            transport,
            async move |cx: ConnectionTo<Agent>| {
                if let Err(e) = cx
                    .send_request(InitializeRequest::new(ProtocolVersion::LATEST))
                    .block_task()
                    .await
                {
                    let _ = event_tx.send(AcpEvent::Error(format!("initialize failed: {e}")));
                    return Ok(());
                }

                let provider = cx
                    .send_request(ConfigReadRequest {
                        key: "GOOSE_PROVIDER".to_string(),
                        is_secret: false,
                    })
                    .block_task()
                    .await;
                let has_provider = match &provider {
                    Ok(r) => r
                        .value
                        .as_str()
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false),
                    Err(_) => false,
                };

                if !has_provider {
                    let _ = event_tx.send(AcpEvent::NeedOnboarding);
                } else if let Err(e) = create_session(&cx, &event_tx).await {
                    let _ = event_tx.send(AcpEvent::Error(format!(
                        "no provider configured or session failed: {e}. Run `goose configure`."
                    )));
                }

                let mut session_id: Option<String> = None;
                loop {
                    tokio::select! {
                        maybe_text = prompt_rx.recv() => {
                            let text = match maybe_text {
                                Some(t) => t,
                                None => break,
                            };
                            if let Some(sid) = &session_id {
                                let resp = cx.send_request(PromptRequest::new(
                                    sid.clone(),
                                    vec![ContentBlock::Text(TextContent::new(text))],
                                )).block_task().await;
                                match resp {
                                    Ok(r) => {
                                        let _ = event_tx.send(AcpEvent::Stopped {
                                            stop_reason: format!("{:?}", r.stop_reason),
                                        });
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(AcpEvent::Error(format!("{e}")));
                                    }
                                }
                            }
                        }
                        maybe_ctrl = control_rx.recv() => {
                            let ctrl = match maybe_ctrl {
                                Some(c) => c,
                                None => break,
                            };
                            match ctrl {
                                Control::NewSession => {
                                    match create_session(&cx, &event_tx).await {
                                        Ok(sid) => session_id = Some(sid),
                                        Err(e) => {
                                            let _ = event_tx.send(AcpEvent::Error(format!("{e}")));
                                        }
                                    }
                                }
                                Control::ConfigReadAll => {
                                    let r = cx.send_request(ConfigReadAllRequest {}).block_task().await;
                                    match r {
                                        Ok(snap) => {
                                            let _ = event_tx.send(AcpEvent::ConfigSnapshot(snap));
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(AcpEvent::Error(format!("{e}")));
                                        }
                                    }
                                }
                                Control::ConfigUpsert { key, value } => {
                                    let r = cx.send_request(ConfigUpsertRequest {
                                        key,
                                        value,
                                        is_secret: false,
                                    }).block_task().await;
                                    let _ = event_tx.send(AcpEvent::OpResult(r.map(|_| ()).map_err(|e| e.to_string())));
                                }
                                Control::ListExtensions => {
                                    let r = cx.send_request(GetConfigExtensionsRequest {}).block_task().await;
                                    match r {
                                        Ok(resp) => {
                                            let _ = event_tx.send(AcpEvent::Extensions(resp));
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(AcpEvent::Error(format!("{e}")));
                                        }
                                    }
                                }
                                Control::ListAvailableExtensions => {
                                    let r = cx.send_request(GetAvailableExtensionsRequest {}).block_task().await;
                                    match r {
                                        Ok(resp) => {
                                            let _ = event_tx.send(AcpEvent::AvailableExtensions(resp));
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(AcpEvent::Error(format!("{e}")));
                                        }
                                    }
                                }
                                Control::AddExtension { extension } => {
                                    let r = cx.send_request(AddConfigExtensionRequest {
                                        extension,
                                        enabled: true,
                                    }).block_task().await;
                                    let _ = event_tx.send(AcpEvent::OpResult(r.map(|_| ()).map_err(|e| e.to_string())));
                                }
                                Control::RemoveExtension { config_key } => {
                                    let r = cx.send_request(RemoveConfigExtensionRequest { config_key }).block_task().await;
                                    let _ = event_tx.send(AcpEvent::OpResult(r.map(|_| ()).map_err(|e| e.to_string())));
                                }
                                Control::SetExtensionEnabled { config_key, enabled } => {
                                    let r = cx.send_request(SetConfigExtensionEnabledRequest {
                                        config_key,
                                        enabled,
                                    }).block_task().await;
                                    let _ = event_tx.send(AcpEvent::OpResult(r.map(|_| ()).map_err(|e| e.to_string())));
                                }
                            }
                        }
                    }
                }
                Ok(())
            },
        );

    // The ACP connection future is `!Send`, so it must run on a LocalSet via
    // spawn_local. The caller is responsible for providing that context.
    tokio::task::spawn_local(async move {
        if let Err(e) = connect_fut.await {
            let _ = event_tx_err.send(AcpEvent::Error(format!("acp connection error: {e}")));
        }
        let _ = child.kill().await;
    });

    (
        AcpClient {
            prompt_tx,
            control_tx,
        },
        events,
    )
}

/// Create a new session in the given working directory and announce readiness.
async fn create_session(
    cx: &ConnectionTo<Agent>,
    event_tx: &mpsc::UnboundedSender<AcpEvent>,
) -> anyhow::Result<String> {
    let session = cx
        .send_request(NewSessionRequest::new(
            std::env::current_dir().map_err(|e| anyhow::anyhow!(e))?,
        ))
        .block_task()
        .await?;
    let session_id = session.session_id.to_string();
    let _ = event_tx.send(AcpEvent::Ready {
        session_id: session_id.clone(),
    });
    Ok(session_id)
}
