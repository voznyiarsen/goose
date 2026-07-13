//! Interactive TUI application state and rendering.
//!
//! One logical content line maps to one rendered line; the viewport shows a
//! tail of the current turn and truncates lines to width so manual scroll math
//! stays correct.

use crate::acp::{AcpClient, AcpEvent, ToolCallView};
use crate::content::{
    render_markdown, truncate_line, CRANBERRY, ERROR_COLOR, GOLD, RULE_COLOR, TEAL, TEXT_DIM,
    TEXT_PRIMARY,
};
use agent_client_protocol::schema::v1::McpServer;
use goose_sdk_types::custom_requests::GooseExtension;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use ratatui_textarea::TextArea;
use std::collections::HashMap;
use std::env;
use std::io::Write;
use tokio::sync::mpsc;

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const INPUT_MAX_ROWS: usize = 8;
const SCROLL_STEP: usize = 3;

/// Curated set of providers offered in the configure overlay. The full setup
/// (OAuth, keys) is handled by `goose configure`; this lets you switch among
/// already-configured providers quickly.
const PROVIDERS: &[&str] = &[
    "openai",
    "anthropic",
    "gemini",
    "groq",
    "ollama",
    "databricks",
    "azure",
    "bedrock",
    "vertexai",
    "mistral",
];

fn default_model(provider: &str) -> &'static str {
    match provider {
        "openai" => "gpt-4o",
        "anthropic" => "claude-sonnet-4-0",
        "gemini" => "gemini-1.5-pro",
        "groq" => "llama-3.1-70b-versatile",
        "ollama" => "llama3.1",
        "databricks" => "databricks-claude-3-7-sonnet",
        "azure" => "gpt-4o",
        "bedrock" => "anthropic.claude-v2",
        "vertexai" => "gemini-1.5-pro",
        "mistral" => "mistral-large-latest",
        _ => "",
    }
}

fn ext_name(ext: &GooseExtension) -> String {
    match ext {
        GooseExtension::Builtin { name, .. } => name.clone(),
        GooseExtension::Platform { name, .. } => name.clone(),
        GooseExtension::Mcp { server, .. } => match server {
            McpServer::Http(h) => h.name.clone(),
            McpServer::Sse(s) => s.name.clone(),
            McpServer::Stdio(s) => s.name.clone(),
            _ => String::new(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Onboarding,
    Configure,
    Extensions,
}

#[derive(Debug, Clone)]
struct ExtensionRow {
    name: String,
    config_key: String,
    enabled: bool,
}

#[derive(Debug)]
enum TurnItem {
    Text(String),
    ToolCall(ToolCallView),
    Error(String),
}

struct Turn {
    user_text: String,
    items: Vec<TurnItem>,
    tool_index: HashMap<String, usize>,
}

pub struct App {
    turns: Vec<Turn>,
    loading: bool,
    status: String,
    ready: bool,
    banner: bool,
    scroll: usize,
    selected_tc: Option<usize>,
    expanded: Option<usize>,
    expanded_scroll: usize,
    should_exit: bool,
    textarea: TextArea<'static>,
    spin_idx: usize,
    initial_prompt: Option<String>,
    queued: Vec<String>,
    client: AcpClient,
    overlay: Overlay,
    need_onboarding: bool,
    provider: String,
    model: String,
    cfg_selected: usize,
    ext_entries: Vec<ExtensionRow>,
    ext_selected: usize,
    ext_avail: Vec<GooseExtension>,
    pending_configure: bool,
}

impl App {
    pub fn new(initial_prompt: String, client: AcpClient) -> Self {
        let initial_prompt = if initial_prompt.is_empty() {
            None
        } else {
            Some(initial_prompt)
        };
        App {
            turns: Vec::new(),
            loading: true,
            status: "connecting…".into(),
            ready: false,
            banner: true,
            scroll: 0,
            selected_tc: None,
            expanded: None,
            expanded_scroll: 0,
            should_exit: false,
            textarea: TextArea::default(),
            spin_idx: 0,
            initial_prompt,
            queued: Vec::new(),
            client,
            overlay: Overlay::None,
            need_onboarding: false,
            provider: String::new(),
            model: String::new(),
            cfg_selected: 0,
            ext_entries: Vec::new(),
            ext_selected: 0,
            ext_avail: Vec::new(),
            pending_configure: false,
        }
    }

    pub fn handle_event(&mut self, ev: AcpEvent) {
        match ev {
            AcpEvent::Ready { .. } => {
                self.ready = true;
                self.loading = false;
                self.need_onboarding = false;
                self.status = "ready".into();
                if self.overlay == Overlay::Onboarding {
                    self.overlay = Overlay::None;
                }
                if let Some(p) = self.initial_prompt.take() {
                    self.start_turn(p);
                }
            }
            AcpEvent::NeedOnboarding => {
                self.need_onboarding = true;
                self.overlay = Overlay::Onboarding;
            }
            AcpEvent::AgentChunk(text) => self.append_agent(&text),
            AcpEvent::ToolCall(tc) => self.add_tool_call(tc),
            AcpEvent::ToolCallUpdate {
                id,
                title,
                status,
                raw_input,
                raw_output,
            } => self.update_tool_call(&id, title, status, raw_input, raw_output),
            AcpEvent::ConfigSnapshot(snap) => {
                if let Some(v) = snap.config.get("GOOSE_PROVIDER") {
                    self.provider = v.as_str().unwrap_or("").to_string();
                }
                if let Some(v) = snap.config.get("GOOSE_MODEL") {
                    self.model = v.as_str().unwrap_or("").to_string();
                }
            }
            AcpEvent::Extensions(resp) => {
                self.ext_entries = resp
                    .extensions
                    .iter()
                    .filter_map(|e| {
                        let cfg_key = e.config_key.clone()?;
                        Some(ExtensionRow {
                            name: ext_name(&e.extension),
                            config_key: cfg_key,
                            enabled: e.enabled,
                        })
                    })
                    .collect();
                self.ext_selected = 0;
            }
            AcpEvent::AvailableExtensions(resp) => {
                self.ext_avail = resp.extensions;
            }
            AcpEvent::OpResult(r) => {
                if let Err(e) = r {
                    self.status = format!("error: {e}");
                } else if self.overlay == Overlay::Extensions {
                    self.client.list_extensions();
                } else if self.overlay == Overlay::Configure {
                    self.client.new_session();
                    self.overlay = Overlay::None;
                }
            }
            AcpEvent::Stopped { stop_reason } => {
                self.loading = false;
                self.status = if stop_reason.contains("EndTurn") {
                    "ready".into()
                } else {
                    stop_reason
                };
                if let Some(next) = self.queued.first().cloned() {
                    self.queued.remove(0);
                    self.start_turn(next);
                }
            }
            AcpEvent::Error(e) => {
                self.loading = false;
                self.status = "error".into();
                self.append_error(e);
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_exit = true;
            return;
        }
        if self.overlay != Overlay::None {
            self.handle_overlay_key(key);
            return;
        }
        if key.code == KeyCode::Esc && self.expanded.is_some() {
            self.expanded = None;
            return;
        }
        if key.code == KeyCode::Esc {
            self.should_exit = true;
            return;
        }
        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.open_configure();
            return;
        }
        if key.code == KeyCode::Char('m') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.open_configure();
            return;
        }
        if key.code == KeyCode::Char('e') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.open_extensions();
            return;
        }
        if self.expanded.is_some() {
            match key.code {
                KeyCode::Up => self.expanded_scroll = self.expanded_scroll.saturating_add(1),
                KeyCode::Down => self.expanded_scroll = self.expanded_scroll.saturating_sub(1),
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Char(' ') && self.selected_tc.is_some() {
            self.expanded = self.selected_tc;
            self.expanded_scroll = 0;
            return;
        }
        match key.code {
            KeyCode::Up => {
                if let Some(sel) = self.selected_tc {
                    if let Some(next) = self.next_tool_call(sel) {
                        self.selected_tc = Some(next);
                    } else {
                        self.scroll_up();
                    }
                } else {
                    self.scroll_up();
                }
            }
            KeyCode::Down => {
                if let Some(sel) = self.selected_tc {
                    if let Some(prev) = self.prev_tool_call(sel) {
                        self.selected_tc = Some(prev);
                    } else {
                        self.scroll_down();
                    }
                } else {
                    self.scroll_down();
                }
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let text: String = self.textarea.lines().join("\n");
                self.textarea = TextArea::default();
                self.submit(text);
            }
            _ => {
                self.textarea.input(key);
            }
        }
    }

    fn open_configure(&mut self) {
        self.overlay = Overlay::Configure;
        self.cfg_selected = PROVIDERS
            .iter()
            .position(|p| *p == self.provider)
            .unwrap_or(0);
        self.client.config_read_all();
    }

    fn open_extensions(&mut self) {
        self.overlay = Overlay::Extensions;
        self.client.list_extensions();
        self.client.list_available_extensions();
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) {
        match self.overlay {
            Overlay::None => {}
            Overlay::Onboarding => match key.code {
                KeyCode::Enter => {
                    self.pending_configure = true;
                    self.overlay = Overlay::None;
                }
                KeyCode::Esc => self.should_exit = true,
                _ => {}
            },
            Overlay::Configure => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Up => {
                    self.cfg_selected = self.cfg_selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.cfg_selected =
                        (self.cfg_selected + 1).min(PROVIDERS.len().saturating_sub(1));
                }
                KeyCode::Enter => {
                    if let Some(p) = PROVIDERS.get(self.cfg_selected) {
                        let p = (*p).to_string();
                        let m = default_model(&p).to_string();
                        self.client
                            .config_upsert("GOOSE_PROVIDER", serde_json::Value::String(p.clone()));
                        self.client
                            .config_upsert("GOOSE_MODEL", serde_json::Value::String(m));
                        self.provider = p;
                        self.status = "configuring…".into();
                    }
                }
                _ => {}
            },
            Overlay::Extensions => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Up => {
                    self.ext_selected = self.ext_selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.ext_selected =
                        (self.ext_selected + 1).min(self.ext_entries.len().saturating_sub(1));
                }
                KeyCode::Enter => {
                    if let Some(row) = self.ext_entries.get(self.ext_selected).cloned() {
                        self.client
                            .set_extension_enabled(&row.config_key, !row.enabled);
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(row) = self.ext_entries.get(self.ext_selected).cloned() {
                        self.client.remove_extension(&row.config_key);
                    }
                }
                KeyCode::Char('a') => {
                    for ext in &self.ext_avail {
                        let name = ext_name(ext);
                        if !self.ext_entries.iter().any(|e| e.name == name) {
                            self.client.add_extension(ext.clone());
                            break;
                        }
                    }
                }
                _ => {}
            },
        }
    }

    fn submit(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if text.starts_with('/') {
            self.run_slash(&text);
            return;
        }
        if self.loading {
            self.queued.push(text);
            return;
        }
        self.start_turn(text);
    }

    fn start_turn(&mut self, text: String) {
        self.turns.push(Turn {
            user_text: text.clone(),
            items: Vec::new(),
            tool_index: HashMap::new(),
        });
        self.banner = false;
        self.scroll = 0;
        self.selected_tc = None;
        self.expanded = None;
        self.loading = true;
        self.status = "thinking…".into();
        self.client.send_prompt(&text);
    }

    fn run_slash(&mut self, raw: &str) {
        match raw {
            "/exit" => self.should_exit = true,
            "/clear" => {
                self.turns.clear();
                self.selected_tc = None;
                self.expanded = None;
                self.scroll = 0;
            }
            _ => self.turns.push(Turn {
                user_text: raw.to_string(),
                items: vec![TurnItem::Text(format!("unknown command: {raw}"))],
                tool_index: HashMap::new(),
            }),
        }
    }

    fn append_agent(&mut self, text: &str) {
        if let Some(turn) = self.turns.last_mut() {
            if let Some(TurnItem::Text(last)) = turn.items.last_mut() {
                last.push_str(text);
            } else {
                turn.items.push(TurnItem::Text(text.to_string()));
            }
        }
    }

    fn add_tool_call(&mut self, tc: ToolCallView) {
        if let Some(turn) = self.turns.last_mut() {
            let idx = turn.items.len();
            turn.tool_index.insert(tc.id.clone(), idx);
            turn.items.push(TurnItem::ToolCall(tc));
        }
    }

    fn update_tool_call(
        &mut self,
        id: &str,
        title: Option<String>,
        status: Option<String>,
        raw_input: Option<String>,
        raw_output: Option<String>,
    ) {
        if let Some(turn) = self.turns.last_mut() {
            if let Some(&idx) = turn.tool_index.get(id) {
                if let Some(TurnItem::ToolCall(tc)) = turn.items.get_mut(idx) {
                    if let Some(t) = title {
                        tc.title = t;
                    }
                    if let Some(s) = status {
                        tc.status = s;
                    }
                    if let Some(i) = raw_input {
                        tc.raw_input = Some(i);
                    }
                    if let Some(o) = raw_output {
                        tc.raw_output = Some(o);
                    }
                }
            }
        }
    }

    fn append_error(&mut self, e: String) {
        if let Some(turn) = self.turns.last_mut() {
            turn.items.push(TurnItem::Error(e));
        }
    }

    fn tool_call_indices(&self) -> Vec<usize> {
        self.turns
            .last()
            .map(|t| {
                t.items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| matches!(i, TurnItem::ToolCall(_)))
                    .map(|(idx, _)| idx)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn next_tool_call(&self, cur: usize) -> Option<usize> {
        self.tool_call_indices().into_iter().find(|i| *i > cur)
    }

    fn prev_tool_call(&self, cur: usize) -> Option<usize> {
        self.tool_call_indices()
            .into_iter()
            .rev()
            .find(|i| *i < cur)
    }

    fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_add(SCROLL_STEP);
    }

    fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_sub(SCROLL_STEP);
    }

    pub fn tick(&mut self) {
        self.spin_idx = self.spin_idx.wrapping_add(1);
    }
}

/// Entry point used by the CLI: render the interactive TUI until exit.
pub async fn run_interactive(
    client: AcpClient,
    mut events: mpsc::UnboundedReceiver<AcpEvent>,
    initial_prompt: String,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    ratatui::crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;
    terminal.clear()?;

    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<KeyEvent>();
    tokio::task::spawn_blocking(move || loop {
        if ratatui::crossterm::event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
            if let Ok(ratatui::crossterm::event::Event::Key(k)) = ratatui::crossterm::event::read()
            {
                let _ = key_tx.send(k);
            }
        }
    });

    let mut app = App::new(initial_prompt, client);

    loop {
        terminal.draw(|f| view(&app, f))?;
        tokio::select! {
            ev = events.recv() => {
                match ev {
                    Some(e) => app.handle_event(e),
                    None => {
                        app.status = "disconnected".into();
                        app.loading = false;
                    }
                }
            }
            key = key_rx.recv() => {
                if let Some(k) = key {
                    app.handle_key(k);
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(80)) => {
                app.tick();
            }
        }
        if app.pending_configure {
            app.pending_configure = false;
            if let Err(e) = run_external_configure(&mut terminal) {
                app.status = format!("configure failed: {e}");
            } else {
                app.client.config_read_all();
                app.client.new_session();
            }
        }
        if app.should_exit {
            break;
        }
    }

    disable_raw_mode()?;
    ratatui::crossterm::execute!(std::io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Temporarily drop out of the TUI to run the interactive `goose configure`
/// flow in a normal terminal, then restore the TUI.
fn run_external_configure<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    ratatui::crossterm::execute!(std::io::stdout(), LeaveAlternateScreen)?;
    let _ = std::process::Command::new(env::current_exe()?)
        .arg("configure")
        .status();
    enable_raw_mode()?;
    ratatui::crossterm::execute!(std::io::stdout(), EnterAlternateScreen)?;
    let _ = terminal.clear();
    Ok(())
}

/// Non-interactive one-shot mode: stream a single prompt to stdout and exit.
pub async fn run_one_shot(
    client: AcpClient,
    mut events: mpsc::UnboundedReceiver<AcpEvent>,
    text: &str,
) -> anyhow::Result<()> {
    client.send_prompt(text);
    let mut out = String::new();
    while let Some(ev) = events.recv().await {
        match ev {
            AcpEvent::AgentChunk(s) => out.push_str(&s),
            AcpEvent::ToolCall(tc) => {
                out.push_str(&format!("\n[tool] {}\n", tc.title));
            }
            AcpEvent::Stopped { .. } => break,
            AcpEvent::Error(e) => {
                out.push_str(&format!("\nerror: {e}\n"));
                break;
            }
            _ => {}
        }
    }
    print!("{out}");
    std::io::stdout().flush()?;
    Ok(())
}

pub fn view(app: &App, frame: &mut Frame) {
    let size = frame.area();
    let header_h: u16 = 1;
    let input_h = input_height(app);
    let viewport_h = size.height.saturating_sub(header_h + input_h).max(1);
    let chunks = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(1),
        Constraint::Length(input_h),
    ])
    .split(size);

    let spin = SPINNER[app.spin_idx % SPINNER.len()];
    let status_style = if app.status == "ready" {
        TEAL
    } else if app.status == "error" {
        ERROR_COLOR
    } else {
        TEXT_DIM
    };
    let header = Line::from(vec![
        Span::styled(format!(" {spin} "), Style::default().fg(GOLD)),
        Span::styled(app.status.clone(), Style::default().fg(status_style)),
    ]);
    frame.render_widget(Paragraph::new(header), chunks[0]);

    if app.overlay != Overlay::None {
        render_overlay(frame, chunks[1], app);
        return;
    }

    if app.banner && app.turns.is_empty() {
        render_splash(frame, chunks[1], app);
    } else if let Some(tc_idx) = app.expanded {
        render_expanded(frame, chunks[1], app, tc_idx);
    } else {
        render_viewport(frame, chunks[1], app, viewport_h as usize);
    }

    render_input(frame, chunks[2], app);
}

fn input_height(app: &App) -> u16 {
    let rows = app.textarea.lines().len().clamp(1, INPUT_MAX_ROWS);
    (rows as u16) + 3
}

fn render_splash(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let text = format!("goose — your on-machine AI agent\n\nstatus: {}", app.status);
    let p = Paragraph::new(text).alignment(Alignment::Center);
    frame.render_widget(p, area);
}

fn render_overlay(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GOLD));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    match app.overlay {
        Overlay::Onboarding => render_onboarding(frame, inner, app),
        Overlay::Configure => render_configure(frame, inner, app),
        Overlay::Extensions => render_extensions(frame, inner, app),
        Overlay::None => {}
    }
}

fn render_onboarding(frame: &mut Frame, area: ratatui::layout::Rect, _app: &App) {
    let lines = vec![
        Line::from(Span::styled(
            "No provider is configured.",
            Style::default()
                .fg(ERROR_COLOR)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "[Enter] Run `goose configure` to set up a provider",
            Style::default().fg(TEXT_PRIMARY),
        )),
        Line::from(Span::styled("[Esc]   Exit", Style::default().fg(TEXT_DIM))),
    ];
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_configure(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("provider: {}   model: {}", app.provider, app.model),
        Style::default().fg(TEXT_DIM),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Select a provider (Enter to apply + default model):",
        Style::default().fg(TEXT_PRIMARY),
    )));
    for (i, p) in PROVIDERS.iter().enumerate() {
        let marker = if i == app.cfg_selected { "▸ " } else { "  " };
        let style = if i == app.cfg_selected {
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{p}"), style)));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_extensions(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Enter toggle · d remove · a add available · r refresh · Esc close",
        Style::default().fg(TEXT_DIM),
    )));
    lines.push(Line::from(""));
    if app.ext_entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no extensions configured)",
            Style::default().fg(TEXT_DIM),
        )));
    }
    for (i, row) in app.ext_entries.iter().enumerate() {
        let marker = if i == app.ext_selected { "▸ " } else { "  " };
        let check = if row.enabled { "[x]" } else { "[ ]" };
        let style = if i == app.ext_selected {
            Style::default().fg(GOLD).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{check} {}", row.name),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn build_turn_lines(app: &App, _width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(turn) = app.turns.last() {
        let user = format!("❯ {}", turn.user_text);
        lines.push(Line::from(Span::styled(
            user,
            Style::default().fg(CRANBERRY).add_modifier(Modifier::BOLD),
        )));
        for (idx, item) in turn.items.iter().enumerate() {
            match item {
                TurnItem::Text(t) => {
                    for l in render_markdown(t) {
                        lines.push(l);
                    }
                }
                TurnItem::ToolCall(tc) => {
                    let selected = app.selected_tc == Some(idx);
                    let marker = if selected { "▸ " } else { "  " };
                    let status = tc.status.trim_end_matches("StopReason").trim();
                    let label = format!("{marker}[{status}] {}", tc.title);
                    let style = if selected {
                        Style::default().fg(GOLD).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(TEXT_DIM)
                    };
                    lines.push(Line::from(Span::styled(label, style)));
                }
                TurnItem::Error(e) => {
                    lines.push(Line::from(Span::styled(
                        format!("⚠ {e}"),
                        Style::default().fg(ERROR_COLOR),
                    )));
                }
            }
        }
    }
    lines
}

fn render_viewport(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, viewport_h: usize) {
    let width = area.width as usize;
    let built = build_turn_lines(app, width);
    let lines: Vec<Line<'static>> = built.into_iter().map(|l| truncate_line(l, width)).collect();
    let total = lines.len();
    let max_scroll = total.saturating_sub(viewport_h);
    let scroll = app.scroll.min(max_scroll);
    let start = total.saturating_sub(viewport_h + scroll);
    let end = (start + viewport_h).min(total);
    let visible: Vec<Line<'static>> = lines[start..end].to_vec();
    frame.render_widget(Paragraph::new(Text::from(visible)), area);
}

fn render_input(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(RULE_COLOR));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(&app.textarea, inner);
}

fn colorize_diff_line(l: &str) -> Line<'static> {
    let c = if l.starts_with('+') {
        ratatui::style::Color::Green
    } else if l.starts_with('-') {
        ratatui::style::Color::LightRed
    } else if l.starts_with("@@") {
        TEXT_DIM
    } else {
        TEXT_PRIMARY
    };
    Line::from(Span::styled(l.to_string(), Style::default().fg(c)))
}

fn render_expanded(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, tc_idx: usize) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(turn) = app.turns.last() {
        if let Some(TurnItem::ToolCall(tc)) = turn.items.get(tc_idx) {
            lines.push(Line::from(Span::styled(
                tc.title.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                format!("status: {}", tc.status),
                Style::default().fg(TEXT_DIM),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Input",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )));
            if let Some(inp) = &tc.raw_input {
                for l in inp.lines() {
                    lines.push(Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(TEXT_DIM),
                    )));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Output",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )));
            if let Some(out) = &tc.raw_output {
                for l in out.lines() {
                    lines.push(colorize_diff_line(l));
                }
            }
        }
    }
    let total = lines.len();
    let scroll = app.expanded_scroll.min(total);
    let start = scroll.min(total);
    let end = (start + area.height as usize).min(total);
    let visible: Vec<Line<'static>> = lines[start..end].to_vec();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" tool call ")
        .border_style(Style::default().fg(GOLD));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_app() -> App {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (control, _crx) = mpsc::unbounded_channel();
        let client = AcpClient {
            prompt_tx: tx,
            control_tx: control,
        };
        let mut app = App::new(String::new(), client);
        let mut tool_index = HashMap::new();
        tool_index.insert("1".to_string(), 1);
        app.turns.push(Turn {
            user_text: "hello".to_string(),
            items: vec![
                TurnItem::Text("Hi there".to_string()),
                TurnItem::ToolCall(ToolCallView {
                    id: "1".to_string(),
                    title: "list files".to_string(),
                    status: "Completed".to_string(),
                    kind: Some("builtin".to_string()),
                    raw_input: None,
                    raw_output: None,
                }),
            ],
            tool_index,
        });
        app.banner = false;
        app
    }

    #[test]
    fn renders_without_panic_and_shows_content() {
        let app = sample_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&app, f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(rendered.contains("hello"), "user prompt should render");
        assert!(
            rendered.contains("list files"),
            "tool call title should render"
        );
    }

    #[test]
    fn expanded_view_renders_tool_output() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (control, _crx) = mpsc::unbounded_channel();
        let client = AcpClient {
            prompt_tx: tx,
            control_tx: control,
        };
        let mut app = App::new(String::new(), client);
        let mut tool_index = HashMap::new();
        tool_index.insert("1".to_string(), 0);
        app.turns.push(Turn {
            user_text: "run".to_string(),
            items: vec![TurnItem::ToolCall(ToolCallView {
                id: "1".to_string(),
                title: "edit".to_string(),
                status: "Completed".to_string(),
                kind: Some("builtin".to_string()),
                raw_input: Some("{}".to_string()),
                raw_output: Some("+ added line".to_string()),
            })],
            tool_index,
        });
        app.banner = false;
        app.expanded = Some(0);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&app, f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            rendered.contains("added line"),
            "expanded tool output should render"
        );
    }

    #[test]
    fn configure_overlay_lists_providers() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (control, _crx) = mpsc::unbounded_channel();
        let client = AcpClient {
            prompt_tx: tx,
            control_tx: control,
        };
        let mut app = App::new(String::new(), client);
        app.overlay = Overlay::Configure;
        app.provider = "anthropic".to_string();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| view(&app, f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let rendered: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            rendered.contains("anthropic"),
            "current provider should render"
        );
        assert!(rendered.contains("openai"), "provider list should render");
    }
}
