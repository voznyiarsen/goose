//! Native terminal UI for goose, built into the `goose` binary.
//!
//! This replaces the previous Node/Ink-based TUI. The CLI spawns `goose acp`
//! as a child process and drives it over the Agent Client Protocol; all UI
//! rendering happens in Rust via `ratatui`.

mod acp;
mod app;
mod args;
mod content;

pub use acp::{AcpClient, AcpEvent, ToolCallView};
pub use args::TuiArgs;

use anyhow::Result;

/// Launch the TUI. Must be called from within a Tokio runtime that provides a
/// `LocalSet` context (the ACP connection future is `!Send` and uses
/// `spawn_local`).
pub async fn run(args: TuiArgs) -> Result<()> {
    if let Some(server) = &args.server {
        anyhow::bail!("connecting to a remote ACP server ({server}) is not yet supported");
    }

    let goose_bin = std::env::current_exe()?;
    let (client, events) = acp::connect(goose_bin);

    if let Some(text) = &args.text {
        app::run_one_shot(client, events, text).await
    } else {
        let initial = args.prompt.join(" ");
        app::run_interactive(client, events, initial).await
    }
}
