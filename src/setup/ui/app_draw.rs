//! Drawing the menu screens (task 6.2, design.md §7). Each screen is a list
//! of lines inside a titled frame, scrolled so the cursor stays visible,
//! with the key legend on the bottom line at all times.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Padding, Paragraph};

use super::{App, Screen};
use crate::setup::ui::flow::{AgentState, Choice, FeatureRow, HomeView};
use crate::setup::ui::text;

/// Which part of the content must be visible.
enum Focus {
    /// Keep this line on screen (the cursor).
    Line(usize),
    /// Scroll by this many lines (diffs and results).
    Scroll(usize),
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

/// `text` wrapped to `width`: the first line starts with `first`, the
/// others with `indent` spaces.
fn wrap(text: &str, width: usize, first: &str, indent: usize) -> Vec<String> {
    let lead = text.len() - text.trim_start_matches(' ').len();
    let mut lines = Vec::new();
    let mut line = format!("{first}{}", &text[..lead]);
    let mut empty = true;
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let len = line.chars().count();
        if !empty && len + 1 + word.chars().count() > width {
            lines.push(std::mem::replace(&mut line, " ".repeat(indent)));
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line.push_str(word);
        empty = false;
    }
    lines.push(line);
    lines
}

/// `text` cut to fit `width` columns (with "…") and padded to it.
fn cell(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count < width {
        format!("{text}{}", " ".repeat(width - count))
    } else {
        let cut: String = text.chars().take(width.saturating_sub(2)).collect();
        format!("{cut}… ")
    }
}

fn marker(selected: bool) -> &'static str {
    if selected { "> " } else { "  " }
}

fn checkbox(on: bool) -> &'static str {
    if on { "[x] " } else { "[ ] " }
}

fn push_wrapped(lines: &mut Vec<Line<'static>>, text: &str, width: usize, style: Style) {
    for l in wrap(text, width, "", 0) {
        lines.push(Line::styled(l, style));
    }
}

impl App {
    /// Draws the current screen and the key legend.
    pub fn draw(&self, frame: &mut Frame) {
        let [body, legend] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        let block = Block::bordered()
            .title(format!(" {} ", self.title()))
            .padding(Padding::horizontal(1));
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let (lines, focus) = self.content(usize::from(inner.width));
        let height = usize::from(inner.height).max(1);
        let max = lines.len().saturating_sub(height);
        self.max_scroll.set(max);
        let offset = match focus {
            Focus::Line(line) => (line + 1).saturating_sub(height),
            Focus::Scroll(scroll) => scroll.min(max),
        };
        let offset = u16::try_from(offset).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
        frame.render_widget(Paragraph::new(self.legend()).style(dim()), legend);
    }

    fn title(&self) -> &'static str {
        match self.screen {
            Screen::Home => text::TITLE_HOME,
            Screen::Agents => text::TITLE_AGENTS,
            Screen::Features => text::TITLE_FEATURES,
            Screen::Review => text::TITLE_REVIEW,
            Screen::Progress => text::TITLE_PROGRESS,
        }
    }

    fn legend(&self) -> &'static str {
        match self.screen {
            Screen::Home => text::LEGEND_HOME,
            Screen::Agents | Screen::Features => text::LEGEND,
            Screen::Review if self.diff.is_some() => text::LEGEND_DIFF,
            Screen::Review => text::LEGEND_REVIEW,
            Screen::Progress if self.working => text::LEGEND_WORKING,
            Screen::Progress => text::LEGEND_DONE,
        }
    }

    fn content(&self, width: usize) -> (Vec<Line<'static>>, Focus) {
        let (mut lines, focus) = match self.screen {
            Screen::Home => self.home_lines(width),
            Screen::Agents => self.agent_lines(width),
            Screen::Features => self.feature_lines(width),
            Screen::Review => match self.diff {
                Some(index) => self.diff_lines(index),
                None => self.review_lines(width),
            },
            Screen::Progress => self.progress_lines(width),
        };
        if let Some(note) = self.note {
            lines.push(Line::raw(""));
            push_wrapped(&mut lines, note, width, bold());
        }
        (lines, focus)
    }

    fn home_lines(&self, width: usize) -> (Vec<Line<'static>>, Focus) {
        let home = self.flow.home();
        let mut lines = vec![
            Line::raw(format!("{}{}", cell(&home.installed, 27), home.newest)),
            Line::raw(""),
        ];
        lines.extend(agent_table(&home, width));
        lines.push(Line::raw(""));
        for note in &home.notes {
            push_wrapped(&mut lines, note, width, Style::new());
        }
        if !home.notes.is_empty() {
            lines.push(Line::raw(""));
        }
        let first = lines.len();
        for (i, action) in home.actions.iter().enumerate() {
            let selected = i == self.cursor;
            let style = if selected { bold() } else { Style::new() };
            lines.push(Line::styled(
                format!("{}{}", marker(selected), action.label()),
                style,
            ));
        }
        (lines, Focus::Line(first + self.cursor))
    }

    fn agent_lines(&self, width: usize) -> (Vec<Line<'static>>, Focus) {
        let mut lines = Vec::new();
        for (i, choice) in self.flow.agent_choices().iter().enumerate() {
            let at = marker(i == self.cursor);
            let (line, style) = match choice {
                Choice::Agent {
                    label,
                    available: true,
                    checked,
                    ..
                } => (format!("{at}{}{label}", checkbox(*checked)), Style::new()),
                Choice::Agent { label, .. } => (format!("{at}    {label}"), dim()),
                Choice::Profile { label, chosen, .. } => {
                    let dot = if *chosen { "(•) " } else { "( ) " };
                    (format!("{at}    {dot}{label}"), Style::new())
                }
            };
            for l in wrap(&line, width, "", 8) {
                lines.push(Line::styled(l, style));
            }
        }
        let focus = Self::cursor_line(&lines);
        (lines, focus)
    }

    /// The line the cursor row starts on (rows may wrap).
    fn cursor_line(lines: &[Line<'static>]) -> Focus {
        let line = lines
            .iter()
            .position(|l| l.spans.first().is_some_and(|s| s.content.starts_with("> ")))
            .unwrap_or(0);
        Focus::Line(line)
    }

    fn feature_lines(&self, width: usize) -> (Vec<Line<'static>>, Focus) {
        let mut lines = Vec::new();
        for (i, row) in self.flow.feature_rows().iter().enumerate() {
            feature_row(&mut lines, row, i == self.cursor, width);
        }
        let focus = Self::cursor_line(&lines);
        (lines, focus)
    }

    fn review_lines(&self, width: usize) -> (Vec<Line<'static>>, Focus) {
        let review = self.flow.review();
        let mut lines = Vec::new();
        for line in &review.intro {
            push_wrapped(&mut lines, line, width, bold());
        }
        let mut n = 0;
        for group in &review.groups {
            lines.push(Line::styled(group.file.clone(), bold()));
            for item in &group.items {
                let first = format!("{}{} ", marker(n == self.cursor), item.mark);
                for l in wrap(&item.summary, width, &first, 4) {
                    lines.push(Line::raw(l));
                }
                n += 1;
            }
        }
        if !review.problems.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled(text::REVIEW_PROBLEMS, bold()));
            for p in &review.problems {
                push_wrapped(&mut lines, p, width, Style::new());
            }
        }
        lines.push(Line::raw(""));
        for note in &review.notes {
            push_wrapped(&mut lines, note, width, dim());
        }
        let focus = Self::cursor_line(&lines);
        (lines, focus)
    }

    fn diff_lines(&self, index: usize) -> (Vec<Line<'static>>, Focus) {
        let mut lines = vec![Line::styled(self.flow.diff_file(index), bold())];
        lines.extend(
            self.flow
                .diff(index)
                .lines()
                .map(|l| Line::raw(l.to_owned())),
        );
        (lines, Focus::Scroll(self.scroll))
    }

    fn progress_lines(&self, width: usize) -> (Vec<Line<'static>>, Focus) {
        let mut lines = Vec::new();
        if self.working {
            lines.push(Line::raw(text::CHECKING));
        } else if let Some(progress) = self.flow.progress() {
            for line in progress.all_lines() {
                push_wrapped(&mut lines, &line, width, Style::new());
            }
        }
        (lines, Focus::Scroll(self.scroll))
    }
}

fn agent_table(home: &HomeView, width: usize) -> Vec<Line<'static>> {
    let header = format!(
        "{}{}{}{}",
        cell(text::COL_AGENT, 16),
        cell(text::COL_STATUS, 12),
        cell(text::COL_FEATURES, 22),
        text::COL_CHECK
    );
    let mut lines = vec![Line::styled(header, bold())];
    for row in &home.rows {
        let label = cell(row.agent.label(), 16);
        if let AgentState::NotFound(reason) = &row.state {
            lines.extend(not_found_lines(&label, reason, width));
            continue;
        }
        let status = if row.state == AgentState::Installed {
            text::STATUS_INSTALLED
        } else {
            text::STATUS_NOT_SET_UP
        };
        lines.push(Line::raw(format!(
            "{label}{}{}{}",
            cell(status, 12),
            cell(&row.features, 22),
            row.last_check
        )));
    }
    lines
}

/// "Antigravity     not found   (reason)", with a long reason wrapped under
/// itself at word breaks.
fn not_found_lines(label: &str, reason: &str, width: usize) -> Vec<Line<'static>> {
    let first = format!("{label}{}", cell(text::STATUS_NOT_FOUND, 12));
    let indent = first.chars().count();
    let parts = wrap(&format!("({reason})"), width.saturating_sub(indent), "", 0);
    parts
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let lead = if i == 0 {
                first.clone()
            } else {
                " ".repeat(indent)
            };
            Line::styled(format!("{lead}{part}"), dim())
        })
        .collect()
}

fn feature_row(lines: &mut Vec<Line<'static>>, row: &FeatureRow, selected: bool, width: usize) {
    let at = marker(selected);
    if row.is_heading() {
        lines.push(Line::styled(format!("{at}{}", row.label), bold()));
        return;
    }
    if row.agent.is_none() {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(format!(
        "{at}  {}{}",
        checkbox(row.on),
        row.label
    )));
    for l in wrap(&row.help, width, "        ", 8) {
        lines.push(Line::styled(l, dim()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_reason_wraps_at_word_breaks_under_its_column() {
        let reason = "no Claude Desktop folder was found anywhere in your user profile or app data";
        let lines = not_found_lines(&cell("Claude Desktop", 16), reason, 76);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        assert!(text.len() >= 2, "{text:?}");
        for line in &text {
            assert!(line.chars().count() <= 76, "{line:?}");
        }
        assert!(text[1].starts_with(&" ".repeat(28)), "{text:?}");
        let joined = text.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains(reason), "{joined}");
    }
}
