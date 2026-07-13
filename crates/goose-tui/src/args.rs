//! Command-line arguments for the TUI, mirroring the previous Node launcher.

use clap::Parser;

/// Launch the goose terminal UI.
#[derive(Parser, Debug, Default)]
#[command(name = "goose-tui", bin_name = "goose tui", disable_help_flag = true)]
pub struct TuiArgs {
    /// Connect to an existing ACP server URL instead of spawning `goose acp`.
    #[arg(long, short = 's', global = true)]
    pub server: Option<String>,

    /// Send a single prompt and exit (non-interactive text mode).
    #[arg(long, short = 't', global = true)]
    pub text: Option<String>,

    /// Initial prompt to send on launch.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub prompt: Vec<String>,
}
