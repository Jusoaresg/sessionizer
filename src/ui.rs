use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, FilterTab, Modal, NewSessionFocus, NewSessionModal};
use crate::items::ItemKind;

// Vibrant transparent-friendly color palette (aligned with terminal violet & slate accents)
const COLOR_BORDER: Color = Color::Rgb(100, 116, 139);      // Crisp Slate (#64748b)
const COLOR_BORDER_ACTIVE: Color = Color::Rgb(139, 92, 246);// Vibrant Violet (#8b5cf6)
const COLOR_TEXT: Color = Color::Rgb(209, 213, 219);        // Soft White (#d1d5db)
const COLOR_MUTED: Color = Color::Rgb(148, 163, 184);       // Muted Slate (#94a3b8)
const COLOR_YELLOW: Color = Color::Rgb(253, 224, 71);       // Bright Yellow (#fde047)
const COLOR_CYAN: Color = Color::Rgb(103, 232, 249);        // Bright Cyan (#67e8f9)
const COLOR_GREEN: Color = Color::Rgb(74, 222, 128);        // Mint Green (#4ade80)
const COLOR_ORANGE: Color = Color::Rgb(251, 146, 60);       // Warm Orange (#fb923c)
const COLOR_PURPLE: Color = Color::Rgb(167, 139, 250);      // Lavender (#a78bfa)
const COLOR_BLUE: Color = Color::Rgb(96, 165, 250);         // Sky Blue (#60a5fa)
const COLOR_RED: Color = Color::Rgb(239, 68, 68);           // Coral Red (#ef4444)
const COLOR_SEL_BG: Color = Color::Rgb(46, 16, 101);        // Deep Violet Selection (#2e1065)

pub fn draw(frame: &mut Frame, app: &App) {
    let size = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 1. Top tabs
            Constraint::Min(5),    // 2. Main content (List + Live Preview)
            Constraint::Length(3), // 3. Search bar (BELOW list)
            Constraint::Length(1), // 4. Footer shortcuts
        ])
        .split(size);

    draw_header(frame, app, chunks[0]);
    draw_main_content(frame, app, chunks[1]);
    draw_search_bar(frame, app, chunks[2]);
    draw_footer(frame, app, chunks[3]);

    // Floating Modals
    match &app.modal {
        Modal::None => {}
        Modal::NewSession(modal) => draw_new_session_modal(frame, app, modal),
        Modal::ConfirmKill(modal) => draw_confirm_kill_modal(frame, modal.session_name.as_str()),
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let tab_titles: Vec<Line> = FilterTab::all()
        .iter()
        .map(|tab| {
            let count = count_items_for_tab(app, *tab);
            let text = format!(" {} ({}) ", tab.label(), count);
            Line::from(text)
        })
        .collect();

    let selected_tab_idx = FilterTab::all()
        .iter()
        .position(|&t| t == app.current_tab)
        .unwrap_or(0);

    let tabs_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_BORDER))
        .border_type(BorderType::Rounded)
        .title(" View (Tab / Shift+Tab) ")
        .title_style(Style::default().fg(COLOR_MUTED));

    let tabs = Tabs::new(tab_titles)
        .block(tabs_block)
        .select(selected_tab_idx)
        .style(Style::default().fg(COLOR_MUTED))
        .highlight_style(
            Style::default()
                .fg(COLOR_CYAN)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
        );

    frame.render_widget(tabs, area);
}

fn count_items_for_tab(app: &App, tab: FilterTab) -> usize {
    app.all_candidates
        .iter()
        .filter(|c| match tab {
            FilterTab::All => true,
            FilterTab::Running => c.is_running,
            FilterTab::Projects => matches!(c.kind, ItemKind::Project),
            FilterTab::Worktrees => matches!(c.kind, ItemKind::Worktree { .. }),
            FilterTab::Configs => matches!(c.kind, ItemKind::Config),
        })
        .count()
}

fn draw_main_content(frame: &mut Frame, app: &App, area: Rect) {
    let show_preview_pane = app.show_preview && area.width >= 80;

    let (list_area, preview_area) = if show_preview_pane {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        (h_chunks[0], Some(h_chunks[1]))
    } else {
        (area, None)
    };

    draw_list(frame, app, list_area);

    if let Some(p_area) = preview_area {
        draw_preview(frame, app, p_area);
    }
}

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let items_count = app.filtered_items.len();
    let title = format!(" Results ({}) ", items_count);

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(title)
        .title_style(Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD));

    let inner = list_block.inner(area);
    frame.render_widget(list_block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if app.filtered_items.is_empty() {
        let msg = if app.search_query.is_empty() {
            "  No projects found in configured paths."
        } else {
            "  No matching session. Press Enter or Ctrl+N to create a new session!"
        };

        let bottom_y = inner.y + inner.height.saturating_sub(1);
        let p = Paragraph::new(Span::styled(msg, Style::default().fg(COLOR_YELLOW)));
        frame.render_widget(p, Rect::new(inner.x, bottom_y, inner.width, 1));
        return;
    }

    let height = inner.height as usize;
    let total = app.filtered_items.len();
    let selected = app.selected_index.min(total.saturating_sub(1));

    // Bottom-up viewport scrolling:
    // r = 0 is rendered at the bottom line of the box (closest to the search bar)
    let scroll_offset = if selected < height {
        0
    } else {
        selected - height + 1
    };

    for r in 0..height {
        let item_idx = scroll_offset + r;
        if item_idx >= total {
            break;
        }

        let is_selected = item_idx == selected;
        let filtered = &app.filtered_items[item_idx];
        let c = &filtered.candidate;

        let mut spans = Vec::new();

        // 1. Cursor indicator
        if is_selected {
            spans.push(Span::styled(
                "▶ ",
                Style::default()
                    .fg(COLOR_YELLOW)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw("  "));
        }

        // 2. Running badge
        if c.is_running {
            if c.is_attached {
                spans.push(Span::styled(
                    "● [att] ",
                    Style::default()
                        .fg(COLOR_GREEN)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    "○ [run] ",
                    Style::default()
                        .fg(COLOR_CYAN)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        } else {
            spans.push(Span::raw("        "));
        }

        // 3. Type badge
        match &c.kind {
            ItemKind::Worktree { .. } => {
                spans.push(Span::styled(
                    "[WT]   ",
                    Style::default()
                        .fg(COLOR_ORANGE)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            ItemKind::Config => {
                spans.push(Span::styled("[CFG]  ", Style::default().fg(COLOR_BLUE)));
            }
            ItemKind::Project => {
                spans.push(Span::styled("[PROJ] ", Style::default().fg(COLOR_PURPLE)));
            }
            ItemKind::ActiveSession => {
                spans.push(Span::styled("[TMUX] ", Style::default().fg(COLOR_GREEN)));
            }
        }

        // 4. Display name with matched characters highlighted
        if filtered.match_indices.is_empty() {
            let name_style = if is_selected {
                Style::default()
                    .fg(COLOR_YELLOW)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(COLOR_TEXT)
            };
            spans.push(Span::styled(&c.display_name, name_style));
        } else {
            let match_set: HashSet<usize> =
                filtered.match_indices.iter().copied().collect();
            for (char_idx, ch) in c.display_name.chars().enumerate() {
                let mut s = String::new();
                s.push(ch);
                if match_set.contains(&char_idx) {
                    spans.push(Span::styled(
                        s,
                        Style::default()
                            .fg(COLOR_YELLOW)
                            .add_modifier(Modifier::BOLD)
                            .add_modifier(Modifier::UNDERLINED),
                    ));
                } else if is_selected {
                    spans.push(Span::styled(s, Style::default().fg(COLOR_YELLOW)));
                } else {
                    spans.push(Span::styled(s, Style::default().fg(COLOR_TEXT)));
                }
            }
        }

        // 5. Windows count
        if let Some(w) = c.windows_count {
            spans.push(Span::styled(
                format!(" ({}w)", w),
                Style::default().fg(COLOR_MUTED),
            ));
        }

        // Inverted: line_y places r=0 at the bottom of the box
        let line_y = inner.y + (inner.height - 1 - r as u16);
        let line_area = Rect::new(inner.x, line_y, inner.width, 1);

        let row_style = if is_selected {
            Style::default().bg(COLOR_SEL_BG)
        } else {
            Style::default()
        };

        let p = Paragraph::new(Line::from(spans)).style(row_style);
        frame.render_widget(p, line_area);
    }
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(" Preview (F1 to Toggle) ")
        .title_style(Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD));

    let inner = preview_block.inner(area);
    frame.render_widget(preview_block, area);

    let Some(candidate) = app.selected_item() else {
        frame.render_widget(Paragraph::new(" No item selected").style(Style::default().fg(COLOR_MUTED)), inner);
        return;
    };

    let mut lines = Vec::new();

    // 1. Session Name
    lines.push(Line::from(vec![
        Span::styled(
            "Session: ",
            Style::default()
                .fg(COLOR_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &candidate.session_name,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // 2. Status
    let status_span = if candidate.is_running {
        if candidate.is_attached {
            Span::styled(
                "● Active (Attached to client)",
                Style::default()
                    .fg(COLOR_GREEN)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                "○ Active (Detached in background)",
                Style::default()
                    .fg(COLOR_CYAN)
                    .add_modifier(Modifier::BOLD),
            )
        }
    } else {
        Span::styled("○ Idle (Not running)", Style::default().fg(COLOR_MUTED))
    };
    lines.push(Line::from(vec![
        Span::styled(
            "Status:  ",
            Style::default()
                .fg(COLOR_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        status_span,
    ]));

    // 3. Path
    lines.push(Line::from(vec![
        Span::styled(
            "Path:    ",
            Style::default()
                .fg(COLOR_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            candidate.path.to_string_lossy().to_string(),
            Style::default().fg(COLOR_TEXT),
        ),
    ]));

    // 4. Git Info
    if let Some(branch) = &app.cached_preview.git_branch {
        lines.push(Line::from(vec![
            Span::styled(
                "Branch:  ",
                Style::default()
                    .fg(COLOR_CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().fg(COLOR_ORANGE)),
            Span::styled(
                branch,
                Style::default()
                    .fg(COLOR_ORANGE)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        if let Some(commit) = &app.cached_preview.git_last_commit {
            lines.push(Line::from(vec![
                Span::styled(
                    "Commit:  ",
                    Style::default()
                        .fg(COLOR_CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(commit, Style::default().fg(COLOR_MUTED)),
            ]));
        }
    }

    lines.push(Line::from(""));

    // 5. Tmux Windows or Directory contents
    if candidate.is_running && !app.cached_preview.tmux_windows.is_empty() {
        lines.push(Line::from(Span::styled(
            "Active Windows:",
            Style::default()
                .fg(COLOR_YELLOW)
                .add_modifier(Modifier::BOLD),
        )));
        for w in &app.cached_preview.tmux_windows {
            let is_active_win = w.contains('●');
            let style = if is_active_win {
                Style::default().fg(COLOR_GREEN).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(COLOR_TEXT)
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(w, style),
            ]));
        }
    } else if !app.cached_preview.dir_entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "Directory Contents:",
            Style::default()
                .fg(COLOR_YELLOW)
                .add_modifier(Modifier::BOLD),
        )));
        for entry in &app.cached_preview.dir_entries {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(entry, Style::default().fg(COLOR_TEXT)),
            ]));
        }
    }

    let p = Paragraph::new(lines);
    frame.render_widget(p, inner);
}

fn draw_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER_ACTIVE))
        .title("  Search Projects & Sessions (Ctrl+N to Create New) ")
        .title_style(Style::default().fg(COLOR_PURPLE).add_modifier(Modifier::BOLD));

    let input_line = Line::from(vec![
        Span::styled(
            " > ",
            Style::default()
                .fg(COLOR_YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &app.search_query,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("█", Style::default().fg(COLOR_CYAN)),
    ]);

    let input_p = Paragraph::new(input_line).block(search_block);
    frame.render_widget(input_p, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(status) = app.active_status() {
        let p = Paragraph::new(Span::styled(
            format!(" ℹ {}", status),
            Style::default()
                .fg(COLOR_YELLOW)
                .add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(p, area);
        return;
    }

    let shortcuts = Line::from(vec![
        Span::styled(" ↵ ", Style::default().fg(COLOR_YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled("Open/Switch  ", Style::default().fg(COLOR_MUTED)),
        Span::styled("^N ", Style::default().fg(COLOR_CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("New Session  ", Style::default().fg(COLOR_MUTED)),
        Span::styled("^X ", Style::default().fg(COLOR_RED).add_modifier(Modifier::BOLD)),
        Span::styled("Kill Session  ", Style::default().fg(COLOR_MUTED)),
        Span::styled("Tab ", Style::default().fg(COLOR_YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled("Filter Tab  ", Style::default().fg(COLOR_MUTED)),
        Span::styled("F1 ", Style::default().fg(COLOR_CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("Preview  ", Style::default().fg(COLOR_MUTED)),
        Span::styled("^R ", Style::default().fg(COLOR_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("Refresh  ", Style::default().fg(COLOR_MUTED)),
        Span::styled("Esc ", Style::default().fg(COLOR_TEXT).add_modifier(Modifier::BOLD)),
        Span::styled("Quit", Style::default().fg(COLOR_MUTED)),
    ]);

    let p = Paragraph::new(shortcuts);
    frame.render_widget(p, area);
}

fn draw_new_session_modal(frame: &mut Frame, app: &App, modal: &NewSessionModal) {
    let area = centered_rect(65, 45, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(COLOR_BORDER_ACTIVE))
        .title(" ✨ Create New Session ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Session Name
            Constraint::Length(1), // Spacer
            Constraint::Length(7), // Directory options
            Constraint::Length(2), // Git init toggle
            Constraint::Min(1),    // Hotkeys / hints
        ])
        .split(inner);

    // 1. Session Name
    let is_name_focused = modal.focus == NewSessionFocus::SessionName;
    let name_spans = if is_name_focused {
        vec![
            Span::styled("Session Name: ", Style::default().fg(COLOR_YELLOW).add_modifier(Modifier::BOLD)),
            Span::styled(&modal.session_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("█", Style::default().fg(COLOR_CYAN)),
            Span::styled(" (Typing edits name)", Style::default().fg(COLOR_MUTED)),
        ]
    } else {
        vec![
            Span::styled("Session Name: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(&modal.session_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" (Press ↑ to edit)", Style::default().fg(COLOR_MUTED)),
        ]
    };
    frame.render_widget(Paragraph::new(Line::from(name_spans)), chunks[0]);

    // 2. Directory Options
    let base_dir = app.config.new_session_base_dir.join(&modal.session_name);
    let cur_dir = std::env::current_dir().unwrap_or_default();
    let tmp_dir = std::env::temp_dir().join(&modal.session_name);

    let custom_path_display = if modal.focus == NewSessionFocus::CustomPathInput {
        format!("4. Custom Path:   {}█", modal.custom_path)
    } else {
        format!("4. Custom Path:   {}", modal.custom_path)
    };

    let options = [
        format!("1. Default Base:   {}", base_dir.display()),
        format!("2. Current PWD:    {}", cur_dir.display()),
        format!("3. Scratch /tmp:   {}", tmp_dir.display()),
        custom_path_display,
    ];

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, opt)| {
            let is_sel = idx == modal.selected_option && modal.focus != NewSessionFocus::SessionName;
            let style = if is_sel {
                Style::default()
                    .fg(COLOR_YELLOW)
                    .bg(COLOR_SEL_BG)
                    .add_modifier(Modifier::BOLD)
            } else if idx == modal.selected_option {
                Style::default().fg(COLOR_YELLOW)
            } else {
                Style::default().fg(COLOR_TEXT)
            };

            let prefix = if is_sel { "▶ " } else { "  " };
            ListItem::new(format!("{}{}", prefix, opt)).style(style)
        })
        .collect();

    let list_title = if modal.focus == NewSessionFocus::FolderSelection {
        "Choose Folder Location: (1-4 or ↑/↓)"
    } else {
        "Choose Folder Location:"
    };

    let list = List::new(items).block(Block::default().title(list_title).title_style(Style::default().fg(COLOR_MUTED)));
    frame.render_widget(list, chunks[2]);

    // 3. Git Init
    let git_check = if modal.init_git { "[✔] Yes" } else { "[ ] No" };
    let git_line = Line::from(vec![
        Span::raw("Initialize git repository: "),
        Span::styled(
            git_check,
            Style::default()
                .fg(if modal.init_git { COLOR_GREEN } else { COLOR_MUTED })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" (Press 'g' to toggle)", Style::default().fg(COLOR_MUTED)),
    ]);
    frame.render_widget(Paragraph::new(git_line), chunks[3]);

    // 4. Modal footer
    let footer = Line::from(vec![
        Span::styled(" ↵ / Enter ", Style::default().fg(COLOR_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("Create   ", Style::default().fg(COLOR_MUTED)),
        Span::styled("Tab / ↓ ", Style::default().fg(COLOR_CYAN).add_modifier(Modifier::BOLD)),
        Span::styled("Focus Folder   ", Style::default().fg(COLOR_MUTED)),
        Span::styled("1-4 ", Style::default().fg(COLOR_YELLOW).add_modifier(Modifier::BOLD)),
        Span::styled("Pick   ", Style::default().fg(COLOR_MUTED)),
        Span::styled("g ", Style::default().fg(COLOR_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("Git   ", Style::default().fg(COLOR_MUTED)),
        Span::styled("Esc ", Style::default().fg(COLOR_RED).add_modifier(Modifier::BOLD)),
        Span::styled("Cancel", Style::default().fg(COLOR_MUTED)),
    ]);
    frame.render_widget(Paragraph::new(footer), chunks[4]);
}

fn draw_confirm_kill_modal(frame: &mut Frame, session_name: &str) {
    let area = centered_rect(50, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(COLOR_RED))
        .title(" ⚠️ Kill Tmux Session ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(2)])
        .split(inner);

    let text = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("Are you sure you want to kill session "),
            Span::styled(
                session_name,
                Style::default()
                    .fg(COLOR_YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("?"),
        ]),
    ])
    .alignment(Alignment::Center);

    let buttons = Paragraph::new(Line::from(vec![
        Span::styled(" [y / ↵] Confirm ", Style::default().fg(COLOR_RED).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled(" [n / Esc] Cancel ", Style::default().fg(COLOR_TEXT)),
    ]))
    .alignment(Alignment::Center);

    frame.render_widget(text, chunks[0]);
    frame.render_widget(buttons, chunks[1]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
