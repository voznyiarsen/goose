use anyhow::Result;
use clap::Parser;
use tokio::task::LocalSet;

/// Launch the goose terminal UI.
///
/// The TUI is implemented in Rust (the `goose-tui` crate) and is built into
/// this binary. It spawns `goose acp` as a child process and communicates
/// with it over stdio via the Agent Client Protocol.
pub async fn handle_tui(args: Vec<String>) -> Result<()> {
    let tui_args = match goose_tui::TuiArgs::try_parse_from(
        std::iter::once("goose-tui".to_string()).chain(args),
    ) {
        Ok(a) => a,
        Err(e) => {
            // clap errors (e.g. --help) are user-facing; print and return.
            eprintln!("{e}");
            return Ok(());
        }
    };

    // The ACP connection future is `!Send`, so it runs on a LocalSet via
    // `spawn_local`. The caller (`run`) is already inside a Tokio runtime.
    let local = LocalSet::new();
    local.run_until(goose_tui::run(tui_args)).await
}
