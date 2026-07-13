//! Text rendering helpers: color palette, lightweight markdown, and truncation.
//!
//! The viewport renders one content line per logical line and truncates to the
//! available width (truncate, never wrap, inside fixed areas) so manual scroll
//! math stays correct.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub const CRANBERRY: Color = Color::Magenta;
pub const TEAL: Color = Color::Cyan;
pub const GOLD: Color = Color::Yellow;
pub const TEXT_PRIMARY: Color = Color::Reset;
pub const TEXT_DIM: Color = Color::DarkGray;
pub const RULE_COLOR: Color = Color::DarkGray;
pub const ERROR_COLOR: Color = Color::LightRed;

/// Render a limited markdown subset (headings, lists, code fences, inline
/// bold/italic/code) into terminal lines. Lines are intentionally not wrapped;
/// callers truncate to the viewport width.
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code = false;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            lines.push(Line::from(Span::styled(
                "···",
                Style::default().fg(TEXT_DIM),
            )));
            continue;
        }
        if in_code {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(TEXT_DIM),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(TEXT_PRIMARY),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let mut spans = vec![Span::styled("• ", Style::default().fg(TEAL))];
            spans.extend(render_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }
        lines.push(Line::from(render_inline(line)));
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

/// Render inline emphasis (`**bold**`, `*italic*`, `` `code` ``) into spans.
fn render_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(idx) = rest.find("**") {
            let (before, after) = rest.split_at(idx);
            if !before.is_empty() {
                spans.push(Span::raw(before.to_string()));
            }
            let (_, after2) = after.split_at(2);
            if let Some(end) = after2.find("**") {
                let (bold, remain) = after2.split_at(end);
                spans.push(Span::styled(
                    bold.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                let (_, next) = remain.split_at(2);
                rest = next;
            } else {
                spans.push(Span::raw("**".to_string()));
                rest = after2;
            }
        } else if let Some(idx) = rest.find('`') {
            let (before, after) = rest.split_at(idx);
            if !before.is_empty() {
                spans.push(Span::raw(before.to_string()));
            }
            let (_, after2) = after.split_at(1);
            if let Some(end) = after2.find('`') {
                let (code, remain) = after2.split_at(end);
                spans.push(Span::styled(code.to_string(), Style::default().fg(TEAL)));
                let (_, next) = remain.split_at(1);
                rest = next;
            } else {
                spans.push(Span::raw("`".to_string()));
                rest = after2;
            }
        } else if let Some(idx) = rest.find('*') {
            let (before, after) = rest.split_at(idx);
            if !before.is_empty() {
                spans.push(Span::raw(before.to_string()));
            }
            let (_, after2) = after.split_at(1);
            if let Some(end) = after2.find('*') {
                let (italic, remain) = after2.split_at(end);
                spans.push(Span::styled(
                    italic.to_string(),
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
                let (_, next) = remain.split_at(1);
                rest = next;
            } else {
                spans.push(Span::raw("*".to_string()));
                rest = after2;
            }
        } else {
            spans.push(Span::raw(rest.to_string()));
            break;
        }
    }
    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

/// Truncate a line to `width` display columns, appending `…` when cut.
pub fn truncate_line(line: Line<'static>, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    let mut used = 0usize;
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut truncated = false;
    for span in line.spans {
        let sw = unicode_width::UnicodeWidthStr::width(span.content.as_ref());
        if used + sw <= width {
            used += sw;
            out.push(span);
        } else {
            let avail = width.saturating_sub(used);
            if avail > 1 {
                let cut = truncate_str(&span.content, avail - 1);
                out.push(Span::styled(format!("{cut}…"), span.style));
            } else {
                out.push(Span::styled("…", span.style));
            }
            truncated = true;
            break;
        }
    }
    if truncated && out.is_empty() {
        out.push(Span::raw("…".to_string()));
    }
    Line::from(out)
}

fn truncate_str(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width {
            break;
        }
        w += cw;
        out.push(c);
    }
    out
}
