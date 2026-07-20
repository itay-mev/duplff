pub mod detail;
pub mod groups;
pub mod help;
pub mod scan;
pub mod state;

use crate::format::human_bytes;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use duplff_core::actions;
use duplff_core::models::ScanConfig;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use state::{AppState, FocusPane, ScanPhase, SortMode};
use std::io::stdout;
use std::sync::mpsc;
use std::time::Duration;

/// Run the interactive TUI.
pub fn run(config: ScanConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = ratatui::init();

    // Start background scan
    let (tx, rx) = mpsc::channel();
    scan::spawn_scan(config, tx);

    let mut state = AppState::scanning();
    let mut should_quit = false;

    while !should_quit {
        // Poll for scan messages
        while let Ok(msg) = rx.try_recv() {
            match msg {
                scan::ScanMessage::ScanProgress(count) => {
                    if let AppState::Scanning {
                        ref mut files_found,
                        ref mut phase,
                        ..
                    } = state
                    {
                        *files_found = count;
                        *phase = ScanPhase::Scanning;
                    }
                }
                scan::ScanMessage::HashProgress { done, total } => {
                    if let AppState::Scanning {
                        ref mut files_hashed,
                        ref mut total_to_hash,
                        ref mut phase,
                        ..
                    } = state
                    {
                        *files_hashed = done;
                        *total_to_hash = total;
                        *phase = ScanPhase::Hashing;
                    }
                }
                scan::ScanMessage::Complete(report) => {
                    state = AppState::into_results(report);
                }
                scan::ScanMessage::Error(msg) => {
                    state = AppState::Error(msg);
                }
            }
        }

        // Render
        terminal.draw(|frame| render(frame, &state))?;

        // Handle input (poll with timeout so we keep processing scan messages)
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                should_quit = handle_input(key, &mut state);
            }
        }
    }

    // Cleanup terminal
    ratatui::restore();
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}

/// Render the current state to the frame.
fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    match state {
        AppState::Scanning {
            files_found,
            files_hashed,
            total_to_hash,
            phase,
        } => {
            render_scanning(
                frame,
                area,
                *phase,
                *files_found,
                *files_hashed,
                *total_to_hash,
            );
        }
        AppState::Results {
            report,
            group_cursor,
            detail_cursor,
            focus,
            marked,
            filter,
            filter_editing,
            sort_mode,
        } => {
            render_results(
                frame,
                area,
                report,
                *group_cursor,
                *detail_cursor,
                *focus,
                marked,
                filter,
                *filter_editing,
                *sort_mode,
            );
        }
        AppState::Confirm {
            report,
            group_cursor,
            detail_cursor,
            focus,
            marked,
            filter,
            sort_mode,
            message,
        } => {
            render_results(
                frame,
                area,
                report,
                *group_cursor,
                *detail_cursor,
                *focus,
                marked,
                filter,
                false,
                *sort_mode,
            );
            render_confirm(frame, area, message);
        }
        AppState::Help {
            report,
            group_cursor,
            detail_cursor,
            focus,
            marked,
            filter,
            sort_mode,
        } => {
            render_results(
                frame,
                area,
                report,
                *group_cursor,
                *detail_cursor,
                *focus,
                marked,
                filter,
                false,
                *sort_mode,
            );
            help::render_help(frame, area);
        }
        AppState::Error(msg) => {
            let paragraph = Paragraph::new(format!("Error: {msg}\n\nPress q to quit."))
                .style(Style::default().fg(Color::Red));
            frame.render_widget(paragraph, area);
        }
    }
}

fn render_scanning(
    frame: &mut Frame,
    area: Rect,
    phase: ScanPhase,
    files_found: usize,
    files_hashed: usize,
    total_to_hash: usize,
) {
    let text = match phase {
        ScanPhase::Scanning => format!("Scanning... {} files found", files_found),
        ScanPhase::Hashing => format!("Hashing... {} / {} files", files_hashed, total_to_hash),
    };
    let paragraph = Paragraph::new(text).style(Style::default().fg(Color::Cyan));
    // Center vertically
    let y = area.height / 2;
    let centered = Rect::new(area.x + 2, y, area.width.saturating_sub(4), 1);
    frame.render_widget(paragraph, centered);
}

#[allow(clippy::too_many_arguments)]
fn render_results(
    frame: &mut Frame,
    area: Rect,
    report: &duplff_core::models::DuplicateReport,
    group_cursor: usize,
    detail_cursor: usize,
    focus: FocusPane,
    marked: &std::collections::HashSet<std::path::PathBuf>,
    filter: &Option<String>,
    filter_editing: bool,
    sort_mode: SortMode,
) {
    // Layout: summary bar (1 line) + groups pane (40%) + detail pane (rest) + help bar (1 line)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Percentage(40),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .split(area);

    // Resolve display order through the same helper as input handling
    let sorted_groups: Vec<&duplff_core::models::DuplicateGroup> =
        state::visible_group_indices(report, filter, sort_mode)
            .into_iter()
            .map(|i| &report.groups[i])
            .collect();

    // Summary bar
    let summary = Line::from(vec![
        Span::styled(
            format!(
                " {} group{}, {} duplicate{}, {} wasted ",
                report.groups.len(),
                if report.groups.len() == 1 { "" } else { "s" },
                report.total_duplicates,
                if report.total_duplicates == 1 {
                    ""
                } else {
                    "s"
                },
                human_bytes(report.total_wasted_bytes),
            ),
            Style::default().fg(Color::White),
        ),
        if !marked.is_empty() {
            Span::styled(
                format!(" | {} marked for deletion", marked.len()),
                Style::default().fg(Color::Red),
            )
        } else {
            Span::raw("")
        },
    ]);
    frame.render_widget(Paragraph::new(summary), chunks[0]);

    // Groups pane
    groups::render_groups(
        frame,
        chunks[1],
        &sorted_groups,
        group_cursor,
        focus == FocusPane::Groups,
        sort_mode.label(),
    );

    // Detail pane
    let current_group = sorted_groups.get(group_cursor).copied();
    detail::render_detail(
        frame,
        chunks[2],
        current_group,
        detail_cursor,
        focus == FocusPane::Detail,
        marked,
    );

    // Help bar — show workflow-aware hints
    if filter_editing {
        let filter_text = filter.as_deref().unwrap_or("");
        let filter_bar = Line::from(vec![
            Span::styled(" /", Style::default().fg(Color::Yellow)),
            Span::raw(filter_text),
            Span::styled("_", Style::default().fg(Color::Yellow)),
            Span::styled(
                "  (Enter:apply  Esc:clear)",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(filter_bar), chunks[3]);
    } else {
        // An applied filter stays visible so users know why groups are hidden
        let filter_hint = match filter {
            Some(f) => vec![
                Span::styled(
                    format!(" [filter: {f}]"),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("  "),
            ],
            None => vec![Span::raw(" ")],
        };
        let mut spans = filter_hint;
        if marked.is_empty() {
            spans.extend([
                Span::styled("Tab", Style::default().fg(Color::Yellow)),
                Span::raw(":switch  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(":select  "),
                Span::styled("Space", Style::default().fg(Color::Yellow)),
                Span::raw(":mark  "),
                Span::styled("D", Style::default().fg(Color::Yellow)),
                Span::raw(":mark-all  "),
                Span::styled("j/k", Style::default().fg(Color::Yellow)),
                Span::raw(":nav  "),
                Span::styled("?", Style::default().fg(Color::Yellow)),
                Span::raw(":help  "),
                Span::styled("q", Style::default().fg(Color::Yellow)),
                Span::raw(":quit"),
            ]);
        } else {
            spans.extend([
                Span::styled("d", Style::default().fg(Color::Red)),
                Span::raw(":trash marked  "),
                Span::styled("u", Style::default().fg(Color::Yellow)),
                Span::raw(":unmark  "),
                Span::styled("Space", Style::default().fg(Color::Yellow)),
                Span::raw(":toggle  "),
                Span::styled("j/k", Style::default().fg(Color::Yellow)),
                Span::raw(":nav  "),
                Span::styled("?", Style::default().fg(Color::Yellow)),
                Span::raw(":help  "),
                Span::styled("q", Style::default().fg(Color::Yellow)),
                Span::raw(":quit"),
            ]);
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::DarkGray)),
            chunks[3],
        );
    }
}

fn render_confirm(frame: &mut Frame, area: Rect, message: &str) {
    use ratatui::widgets::{Block, Borders, Clear};

    let width = 50.min(area.width.saturating_sub(4));
    let height = 5.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let confirm_area = Rect::new(x, y, width, height);

    let block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Yellow).bg(Color::DarkGray));

    let text = format!("{message}\n\nPress y to confirm, n to cancel.");
    let paragraph = Paragraph::new(text).block(block);

    frame.render_widget(Clear, confirm_area);
    frame.render_widget(paragraph, confirm_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use duplff_core::models::{DuplicateGroup, DuplicateReport, FileEntry, KeepReason, RankedFile};
    use std::path::Path;
    use std::time::SystemTime;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ranked(path: &str, size: u64) -> RankedFile {
        RankedFile {
            entry: FileEntry {
                path: path.into(),
                size,
                modified: SystemTime::UNIX_EPOCH,
            },
            reason: KeepReason::LexicographicFirst,
        }
    }

    /// Group 0 wastes few bytes and group 1 wastes many, so the default
    /// WastedDesc sort displays them in the reverse of storage order.
    fn make_report() -> DuplicateReport {
        let g_small = DuplicateGroup {
            hash: [0u8; 32],
            size: 10,
            keep: ranked("/small/keep.txt", 10),
            duplicates: vec![ranked("/small/dup.txt", 10)],
        };
        let g_big = DuplicateGroup {
            hash: [1u8; 32],
            size: 1000,
            keep: ranked("/big/keep.txt", 1000),
            duplicates: vec![ranked("/big/dup.txt", 1000)],
        };
        DuplicateReport {
            groups: vec![g_small, g_big],
            total_files_scanned: 4,
            total_bytes_scanned: 2020,
            total_duplicates: 2,
            total_wasted_bytes: 1010,
        }
    }

    fn type_filter(state: &mut AppState, text: &str) {
        handle_input(key(KeyCode::Char('/')), state);
        for c in text.chars() {
            handle_input(key(KeyCode::Char(c)), state);
        }
        handle_input(key(KeyCode::Enter), state);
    }

    #[test]
    fn marking_acts_on_the_group_shown_at_the_cursor() {
        let mut state = AppState::into_results(make_report());
        // Enter the detail pane of the first displayed group. WastedDesc
        // shows /big first even though it is stored second.
        handle_input(key(KeyCode::Enter), &mut state);
        handle_input(key(KeyCode::Char('D')), &mut state);
        let AppState::Results { marked, .. } = &state else {
            panic!("expected results state");
        };
        assert!(
            marked.contains(Path::new("/big/dup.txt")),
            "mark-all must hit the displayed group, marked: {marked:?}"
        );
        assert!(!marked.contains(Path::new("/small/dup.txt")));
    }

    #[test]
    fn applied_filter_allows_navigation_keys() {
        let mut state = AppState::into_results(make_report());
        type_filter(&mut state, "small");
        // After Enter the filter is applied and j must navigate, not append
        handle_input(key(KeyCode::Char('j')), &mut state);
        let AppState::Results { filter, .. } = &state else {
            panic!("expected results state");
        };
        assert_eq!(filter.as_deref(), Some("small"));
    }

    #[test]
    fn marking_respects_active_filter() {
        let mut state = AppState::into_results(make_report());
        type_filter(&mut state, "small");
        // Only /small is visible, so mark-all at cursor 0 must hit it
        handle_input(key(KeyCode::Enter), &mut state);
        handle_input(key(KeyCode::Char('D')), &mut state);
        let AppState::Results { marked, .. } = &state else {
            panic!("expected results state");
        };
        assert!(marked.contains(Path::new("/small/dup.txt")));
        assert!(!marked.contains(Path::new("/big/dup.txt")));
    }

    #[test]
    fn cursor_clamps_to_visible_groups() {
        let mut state = AppState::into_results(make_report());
        type_filter(&mut state, "small");
        handle_input(key(KeyCode::Char('j')), &mut state);
        let AppState::Results { group_cursor, .. } = &state else {
            panic!("expected results state");
        };
        assert_eq!(*group_cursor, 0, "cursor must clamp to the filtered list");
    }

    #[test]
    fn esc_clears_applied_filter_before_quitting() {
        let mut state = AppState::into_results(make_report());
        type_filter(&mut state, "small");
        let quit = handle_input(key(KeyCode::Esc), &mut state);
        assert!(!quit, "first Esc clears the filter instead of quitting");
        let AppState::Results { filter, .. } = &state else {
            panic!("expected results state");
        };
        assert!(filter.is_none());
        let quit = handle_input(key(KeyCode::Esc), &mut state);
        assert!(quit, "second Esc quits");
    }
}

/// Handle a key event. Returns true if the app should quit.
fn handle_input(key: KeyEvent, state: &mut AppState) -> bool {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    match state {
        AppState::Scanning { .. } => {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                return true;
            }
        }
        AppState::Error(_) => {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                return true;
            }
        }
        AppState::Help {
            report,
            group_cursor,
            detail_cursor,
            focus,
            marked,
            filter,
            sort_mode,
        } => {
            // Any key dismisses help
            let r = std::mem::replace(
                report,
                duplff_core::models::DuplicateReport {
                    groups: vec![],
                    total_files_scanned: 0,
                    total_bytes_scanned: 0,
                    total_duplicates: 0,
                    total_wasted_bytes: 0,
                },
            );
            let gc = *group_cursor;
            let dc = *detail_cursor;
            let f = *focus;
            let m = std::mem::take(marked);
            let flt = std::mem::take(filter);
            let sm = *sort_mode;
            *state = AppState::Results {
                report: r,
                group_cursor: gc,
                detail_cursor: dc,
                focus: f,
                marked: m,
                filter: flt,
                filter_editing: false,
                sort_mode: sm,
            };
        }
        AppState::Confirm {
            report,
            group_cursor,
            detail_cursor,
            focus,
            marked,
            filter,
            sort_mode,
            ..
        } => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Perform deletion
                let groups_to_delete: Vec<_> = report
                    .groups
                    .iter()
                    .map(|g| {
                        let mut g = g.clone();
                        g.duplicates.retain(|d| marked.contains(&d.entry.path));
                        g
                    })
                    .filter(|g| !g.duplicates.is_empty())
                    .collect();

                if let Err(e) = actions::trash_duplicates(&groups_to_delete) {
                    *state = AppState::Error(e.to_string());
                    return false;
                }

                // Remove trashed files from the report
                let deleted_paths: std::collections::HashSet<_> = marked.iter().cloned().collect();
                let mut new_report = std::mem::replace(
                    report,
                    duplff_core::models::DuplicateReport {
                        groups: vec![],
                        total_files_scanned: 0,
                        total_bytes_scanned: 0,
                        total_duplicates: 0,
                        total_wasted_bytes: 0,
                    },
                );
                for group in &mut new_report.groups {
                    group
                        .duplicates
                        .retain(|d| !deleted_paths.contains(&d.entry.path));
                }
                new_report.groups.retain(|g| !g.duplicates.is_empty());
                new_report.total_duplicates =
                    new_report.groups.iter().map(|g| g.duplicates.len()).sum();
                new_report.total_wasted_bytes =
                    new_report.groups.iter().map(|g| g.wasted_bytes()).sum();

                let sm = *sort_mode;
                *state = AppState::Results {
                    report: new_report,
                    group_cursor: 0,
                    detail_cursor: 0,
                    focus: FocusPane::Groups,
                    marked: std::collections::HashSet::new(),
                    filter: None,
                    filter_editing: false,
                    sort_mode: sm,
                };
            }
            _ => {
                // Cancel — go back to results
                let r = std::mem::replace(
                    report,
                    duplff_core::models::DuplicateReport {
                        groups: vec![],
                        total_files_scanned: 0,
                        total_bytes_scanned: 0,
                        total_duplicates: 0,
                        total_wasted_bytes: 0,
                    },
                );
                let gc = *group_cursor;
                let dc = *detail_cursor;
                let f = *focus;
                let m = std::mem::take(marked);
                let flt = std::mem::take(filter);
                let sm = *sort_mode;
                *state = AppState::Results {
                    report: r,
                    group_cursor: gc,
                    detail_cursor: dc,
                    focus: f,
                    marked: m,
                    filter: flt,
                    filter_editing: false,
                    sort_mode: sm,
                };
            }
        },
        AppState::Results {
            report,
            group_cursor,
            detail_cursor,
            focus,
            marked,
            filter,
            filter_editing,
            sort_mode,
        } => {
            // While editing, keystrokes go to the filter text
            if *filter_editing {
                match key.code {
                    KeyCode::Esc => {
                        *filter = None;
                        *filter_editing = false;
                        *group_cursor = 0;
                        *detail_cursor = 0;
                    }
                    KeyCode::Enter => {
                        // Apply the filter and hand keys back to navigation.
                        // An empty filter is cleared entirely.
                        if filter.as_deref().is_none_or(|f| f.is_empty()) {
                            *filter = None;
                        }
                        *filter_editing = false;
                    }
                    KeyCode::Backspace => {
                        if let Some(f) = filter {
                            f.pop();
                            if f.is_empty() {
                                *filter = None;
                                *filter_editing = false;
                            }
                        }
                        *group_cursor = 0;
                        *detail_cursor = 0;
                    }
                    KeyCode::Char(c) => {
                        if let Some(f) = filter {
                            f.push(c);
                        }
                        *group_cursor = 0;
                        *detail_cursor = 0;
                    }
                    _ => {}
                }
                return false;
            }

            // The group cursor is a position in the displayed order, so
            // resolve it to a report index before touching any group.
            let visible = state::visible_group_indices(report, filter, *sort_mode);
            let group_at_cursor = visible.get(*group_cursor).copied();

            match key.code {
                KeyCode::Char('q') => return true,
                KeyCode::Esc => {
                    // Esc clears an applied filter before it quits
                    if filter.is_some() {
                        *filter = None;
                        *group_cursor = 0;
                        *detail_cursor = 0;
                    } else {
                        return true;
                    }
                }
                KeyCode::Char('/') => {
                    if filter.is_none() {
                        *filter = Some(String::new());
                    }
                    *filter_editing = true;
                }
                KeyCode::Char('s') => {
                    *sort_mode = sort_mode.next();
                    *group_cursor = 0;
                    *detail_cursor = 0;
                }
                KeyCode::Char('?') => {
                    let r = std::mem::replace(
                        report,
                        duplff_core::models::DuplicateReport {
                            groups: vec![],
                            total_files_scanned: 0,
                            total_bytes_scanned: 0,
                            total_duplicates: 0,
                            total_wasted_bytes: 0,
                        },
                    );
                    let gc = *group_cursor;
                    let dc = *detail_cursor;
                    let f = *focus;
                    let m = std::mem::take(marked);
                    let flt = std::mem::take(filter);
                    let sm = *sort_mode;
                    *state = AppState::Help {
                        report: r,
                        group_cursor: gc,
                        detail_cursor: dc,
                        focus: f,
                        marked: m,
                        filter: flt,
                        sort_mode: sm,
                    };
                }
                KeyCode::Tab => {
                    *focus = match focus {
                        FocusPane::Groups => FocusPane::Detail,
                        FocusPane::Detail => FocusPane::Groups,
                    };
                }
                KeyCode::Enter => {
                    if *focus == FocusPane::Groups && group_at_cursor.is_some() {
                        *focus = FocusPane::Detail;
                        *detail_cursor = 0;
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => match focus {
                    FocusPane::Groups => {
                        if !visible.is_empty() {
                            *group_cursor = (*group_cursor + 1).min(visible.len() - 1);
                            *detail_cursor = 0;
                        }
                    }
                    FocusPane::Detail => {
                        if let Some(gi) = group_at_cursor {
                            let max = report.groups[gi].duplicates.len(); // 0 = keep, 1..=n = dups
                            *detail_cursor = (*detail_cursor + 1).min(max);
                        }
                    }
                },
                KeyCode::Char('k') | KeyCode::Up => match focus {
                    FocusPane::Groups => {
                        *group_cursor = group_cursor.saturating_sub(1);
                        *detail_cursor = 0;
                    }
                    FocusPane::Detail => {
                        *detail_cursor = detail_cursor.saturating_sub(1);
                    }
                },
                KeyCode::Char(' ') => {
                    if *focus == FocusPane::Detail {
                        if let Some(group) = group_at_cursor.map(|gi| &report.groups[gi]) {
                            // detail_cursor 0 = keep file (can't mark), 1+ = duplicates
                            if *detail_cursor > 0 {
                                let dup_idx = *detail_cursor - 1;
                                if let Some(dup) = group.duplicates.get(dup_idx) {
                                    let path = dup.entry.path.clone();
                                    if marked.contains(&path) {
                                        marked.remove(&path);
                                    } else {
                                        marked.insert(path);
                                    }
                                }
                            }
                        }
                    }
                }
                KeyCode::Char('D') => {
                    if *focus == FocusPane::Detail {
                        if let Some(gi) = group_at_cursor {
                            for dup in &report.groups[gi].duplicates {
                                marked.insert(dup.entry.path.clone());
                            }
                        }
                    }
                }
                KeyCode::Char('u') => {
                    if *focus == FocusPane::Detail {
                        if let Some(gi) = group_at_cursor {
                            for dup in &report.groups[gi].duplicates {
                                marked.remove(&dup.entry.path);
                            }
                        }
                    }
                }
                KeyCode::Char('d') if !marked.is_empty() => {
                    let count = marked.len();
                    let bytes: u64 = report
                        .groups
                        .iter()
                        .flat_map(|g| g.duplicates.iter())
                        .filter(|d| marked.contains(&d.entry.path))
                        .map(|d| d.entry.size)
                        .sum();
                    let message = format!(
                        "Delete {} file{} ({}) to trash?",
                        count,
                        if count == 1 { "" } else { "s" },
                        human_bytes(bytes),
                    );
                    let r = std::mem::replace(
                        report,
                        duplff_core::models::DuplicateReport {
                            groups: vec![],
                            total_files_scanned: 0,
                            total_bytes_scanned: 0,
                            total_duplicates: 0,
                            total_wasted_bytes: 0,
                        },
                    );
                    let gc = *group_cursor;
                    let dc = *detail_cursor;
                    let f = *focus;
                    let m = std::mem::take(marked);
                    let flt = std::mem::take(filter);
                    let sm = *sort_mode;
                    *state = AppState::Confirm {
                        report: r,
                        group_cursor: gc,
                        detail_cursor: dc,
                        focus: f,
                        marked: m,
                        filter: flt,
                        sort_mode: sm,
                        message,
                    };
                }
                _ => {}
            }
        }
    }
    false
}
