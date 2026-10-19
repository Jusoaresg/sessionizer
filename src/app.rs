use std::path::{Path, PathBuf};
use std::process::Command;

use crossterm::event::{KeyCode, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

use crate::config::Config;
use crate::items::{Candidate, ItemKind};
use crate::scanner;
use crate::tmux;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterTab {
    All,
    Running,
    Projects,
    Worktrees,
    Configs,
}

impl FilterTab {
    pub fn all() -> &'static [FilterTab] {
        &[
            FilterTab::All,
            FilterTab::Running,
            FilterTab::Projects,
            FilterTab::Worktrees,
            FilterTab::Configs,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            FilterTab::All => "All",
            FilterTab::Running => "Running",
            FilterTab::Projects => "Projects",
            FilterTab::Worktrees => "Worktrees",
            FilterTab::Configs => "Configs",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            FilterTab::All => FilterTab::Running,
            FilterTab::Running => FilterTab::Projects,
            FilterTab::Projects => FilterTab::Worktrees,
            FilterTab::Worktrees => FilterTab::Configs,
            FilterTab::Configs => FilterTab::All,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            FilterTab::All => FilterTab::Configs,
            FilterTab::Running => FilterTab::All,
            FilterTab::Projects => FilterTab::Running,
            FilterTab::Worktrees => FilterTab::Projects,
            FilterTab::Configs => FilterTab::Worktrees,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilteredItem {
    pub candidate: Candidate,
    pub score: i64,
    pub match_indices: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewSessionFocus {
    SessionName,
    FolderSelection,
    CustomPathInput,
}

#[derive(Debug, Clone)]
pub struct NewSessionModal {
    pub session_name: String,
    pub selected_option: usize,
    pub custom_path: String,
    pub init_git: bool,
    pub focus: NewSessionFocus,
}

#[derive(Debug, Clone)]
pub struct ConfirmKillModal {
    pub session_name: String,
}

#[derive(Debug, Clone)]
pub enum Modal {
    None,
    NewSession(NewSessionModal),
    ConfirmKill(ConfirmKillModal),
}

#[derive(Debug, Clone, Default)]
pub struct PreviewInfo {
    pub tmux_windows: Vec<String>,
    pub git_branch: Option<String>,
    pub git_last_commit: Option<String>,
    pub dir_entries: Vec<String>,
}

pub enum ActionToExecute {
    None,
    Launch {
        session_name: String,
        path: PathBuf,
        is_worktree: bool,
        wt_name: Option<String>,
    },
    Exit,
}

pub struct App {
    pub config: Config,
    pub all_candidates: Vec<Candidate>,
    pub filtered_items: Vec<FilteredItem>,
    pub selected_index: usize,
    pub search_query: String,
    pub current_tab: FilterTab,
    pub show_preview: bool,
    pub status_message: Option<(String, std::time::Instant)>,

    pub modal: Modal,
    pub action: ActionToExecute,

    // Cached preview for current selection
    pub cached_preview_target: Option<String>,
    pub cached_preview: PreviewInfo,

    matcher: SkimMatcherV2,
}

impl App {
    pub fn new(config: Config) -> Self {
        let all_candidates = scanner::scan_all(&config);
        let matcher = SkimMatcherV2::default();

        let mut app = Self {
            config,
            all_candidates,
            filtered_items: Vec::new(),
            selected_index: 0,
            search_query: String::new(),
            current_tab: FilterTab::All,
            show_preview: true,
            status_message: None,
            modal: Modal::None,
            action: ActionToExecute::None,
            cached_preview_target: None,
            cached_preview: PreviewInfo::default(),
            matcher,
        };

        app.apply_filter();
        app.update_preview();
        app
    }

    pub fn rescan(&mut self) {
        self.all_candidates = scanner::scan_all(&self.config);
        self.apply_filter();
        self.set_status("Refreshed projects and sessions");
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), std::time::Instant::now()));
    }

    pub fn active_status(&self) -> Option<&str> {
        if let Some((msg, time)) = &self.status_message {
            if time.elapsed() < std::time::Duration::from_secs(3) {
                return Some(msg.as_str());
            }
        }
        None
    }

    pub fn apply_filter(&mut self) {
        let query = self.search_query.trim().to_lowercase();

        let mut results = Vec::new();

        for candidate in &self.all_candidates {
            // Tab check
            let tab_match = match self.current_tab {
                FilterTab::All => true,
                FilterTab::Running => candidate.is_running,
                FilterTab::Projects => matches!(candidate.kind, ItemKind::Project),
                FilterTab::Worktrees => matches!(candidate.kind, ItemKind::Worktree { .. }),
                FilterTab::Configs => matches!(candidate.kind, ItemKind::Config),
            };

            if !tab_match {
                continue;
            }

            if query.is_empty() {
                results.push(FilteredItem {
                    candidate: candidate.clone(),
                    score: if candidate.is_running { 100 } else { 0 },
                    match_indices: Vec::new(),
                });
            } else {
                // Match against display name or session name
                let text_to_match = format!("{} {}", candidate.display_name, candidate.session_name);
                if let Some((score, indices)) = self.matcher.fuzzy_indices(&text_to_match, &query) {
                    let boost = if candidate.is_running { 50 } else { 0 };
                    results.push(FilteredItem {
                        candidate: candidate.clone(),
                        score: score + boost,
                        match_indices: indices,
                    });
                }
            }
        }

        // Sort by score descending
        if !query.is_empty() {
            results.sort_by(|a, b| b.score.cmp(&a.score));
        }

        self.filtered_items = results;
        self.selected_index = 0;
        self.update_preview();
    }

    pub fn select_next(&mut self) {
        if !self.filtered_items.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.filtered_items.len() - 1);
            self.update_preview();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.filtered_items.is_empty() {
            self.selected_index = self.selected_index.saturating_sub(1);
            self.update_preview();
        }
    }

    pub fn next_tab(&mut self) {
        self.current_tab = self.current_tab.next();
        self.apply_filter();
    }

    pub fn prev_tab(&mut self) {
        self.current_tab = self.current_tab.prev();
        self.apply_filter();
    }

    pub fn toggle_preview(&mut self) {
        self.show_preview = !self.show_preview;
    }

    pub fn selected_item(&self) -> Option<&Candidate> {
        self.filtered_items.get(self.selected_index).map(|item| &item.candidate)
    }

    pub fn update_preview(&mut self) {
        let (session_name, is_running, path) = match self.selected_item() {
            Some(c) => (c.session_name.clone(), c.is_running, c.path.clone()),
            None => {
                self.cached_preview_target = None;
                self.cached_preview = PreviewInfo::default();
                return;
            }
        };

        if self.cached_preview_target.as_deref() == Some(&session_name) {
            return;
        }

        self.cached_preview_target = Some(session_name.clone());

        let mut preview = PreviewInfo::default();

        // If running, query tmux windows
        if is_running {
            preview.tmux_windows = tmux::list_windows(&session_name);
        }

        // Check git info
        preview.git_branch = get_git_branch(&path);
        if preview.git_branch.is_some() {
            preview.git_last_commit = get_git_last_commit(&path);
        }

        // Check directory entries
        if let Ok(entries) = std::fs::read_dir(&path) {
            let mut files: Vec<String> = entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') && name != ".worktrees" {
                        None
                    } else if e.path().is_dir() {
                        Some(format!("📁 {}/", name))
                    } else {
                        Some(format!("📄 {}", name))
                    }
                })
                .take(10)
                .collect();
            files.sort();
            preview.dir_entries = files;
        }

        self.cached_preview = preview;
    }

    pub fn confirm_selection(&mut self) {
        if let Some(candidate) = self.selected_item() {
            self.action = ActionToExecute::Launch {
                session_name: candidate.session_name.clone(),
                path: candidate.path.clone(),
                is_worktree: candidate.is_worktree(),
                wt_name: candidate.wt_name().map(|s| s.to_string()),
            };
        } else if !self.search_query.trim().is_empty() {
            // No matches: open New Session modal with search query
            self.open_new_session_modal(None);
        }
    }

    pub fn open_new_session_modal(&mut self, forced_name: Option<String>) {
        let name = forced_name
            .unwrap_or_else(|| self.search_query.trim().to_string())
            .replace(' ', "-");
        let name = if name.is_empty() {
            "my-session".to_string()
        } else {
            name
        };

        let current_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .to_string();

        self.modal = Modal::NewSession(NewSessionModal {
            session_name: name,
            selected_option: 0,
            custom_path: current_dir,
            init_git: true,
            focus: NewSessionFocus::SessionName,
        });
    }

    pub fn request_kill_selected(&mut self) {
        if let Some(candidate) = self.selected_item() {
            if candidate.is_running {
                self.modal = Modal::ConfirmKill(ConfirmKillModal {
                    session_name: candidate.session_name.clone(),
                });
            } else {
                self.set_status("Session is not currently running");
            }
        }
    }

    pub fn confirm_kill(&mut self) {
        if let Modal::ConfirmKill(ConfirmKillModal { session_name }) = &self.modal {
            if tmux::kill_session(session_name) {
                self.set_status(format!("Killed session '{}'", session_name));
            } else {
                self.set_status(format!("Failed to kill session '{}'", session_name));
            }
        }
        self.modal = Modal::None;
        self.rescan();
    }

    pub fn confirm_new_session(&mut self) {
        let Modal::NewSession(ref mut modal) = self.modal else {
            return;
        };

        let session_name = modal.session_name.trim().replace('.', "_");
        if session_name.is_empty() {
            self.set_status("Session name cannot be empty");
            return;
        }

        let target_path = match modal.selected_option {
            0 => {
                let base = &self.config.new_session_base_dir;
                base.join(&session_name)
            }
            1 => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            2 => std::env::temp_dir().join(&session_name),
            _ => PathBuf::from(modal.custom_path.trim()),
        };

        // Create target directory if it doesn't exist
        if !target_path.exists() {
            if let Err(e) = std::fs::create_dir_all(&target_path) {
                self.set_status(format!("Failed to create directory: {}", e));
                return;
            }
        }

        // Initialize git if requested and not present
        if modal.init_git && !target_path.join(".git").exists() {
            let _ = Command::new("git")
                .args(["init", "-q"])
                .current_dir(&target_path)
                .status();
        }

        let is_wt = false;
        let wt_name = None;

        self.action = ActionToExecute::Launch {
            session_name,
            path: target_path,
            is_worktree: is_wt,
            wt_name,
        };

        self.modal = Modal::None;
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match &mut self.modal {
            Modal::None => self.handle_main_key(code, modifiers),
            Modal::NewSession(_) => self.handle_new_session_modal_key(code, modifiers),
            Modal::ConfirmKill(_) => self.handle_confirm_kill_modal_key(code),
        }
    }

    fn handle_main_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if modifiers.contains(KeyModifiers::CONTROL) {
            match code {
                KeyCode::Char('c') => {
                    self.action = ActionToExecute::Exit;
                }
                KeyCode::Char('n') => {
                    self.open_new_session_modal(None);
                }
                KeyCode::Char('x') | KeyCode::Char('d') => {
                    self.request_kill_selected();
                }
                KeyCode::Char('r') => {
                    self.rescan();
                }
                KeyCode::Char('p') => {
                    self.toggle_preview();
                }
                KeyCode::Char('j') => {
                    self.select_prev();
                }
                KeyCode::Char('k') => {
                    self.select_next();
                }
                KeyCode::Char('t') => {
                    self.next_tab();
                }
                KeyCode::Char('w')
                | KeyCode::Char('h')
                | KeyCode::Backspace
                | KeyCode::Char('\u{17}')
                | KeyCode::Char('\u{8}') => {
                    delete_word(&mut self.search_query);
                    self.apply_filter();
                }
                KeyCode::Char('u') => {
                    self.search_query.clear();
                    self.apply_filter();
                }
                _ => {}
            }
            return;
        }

        if modifiers.contains(KeyModifiers::ALT) && code == KeyCode::Backspace {
            delete_word(&mut self.search_query);
            self.apply_filter();
            return;
        }

        match code {
            KeyCode::Esc => {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.apply_filter();
                } else {
                    self.action = ActionToExecute::Exit;
                }
            }
            KeyCode::Enter => {
                self.confirm_selection();
            }
            KeyCode::Down => {
                self.select_prev();
            }
            KeyCode::Up => {
                self.select_next();
            }
            KeyCode::Tab => {
                self.next_tab();
            }
            KeyCode::BackTab => {
                self.prev_tab();
            }
            KeyCode::F(1) => {
                self.toggle_preview();
            }
            KeyCode::Delete => {
                self.request_kill_selected();
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.apply_filter();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.apply_filter();
            }
            _ => {}
        }
    }

    fn handle_new_session_modal_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.modal = Modal::None;
            return;
        }

        let Modal::NewSession(ref mut modal) = self.modal else {
            return;
        };

        match modal.focus {
            NewSessionFocus::SessionName => {
                if modifiers.contains(KeyModifiers::CONTROL)
                    && (code == KeyCode::Char('w')
                        || code == KeyCode::Char('h')
                        || code == KeyCode::Backspace
                        || code == KeyCode::Char('\u{17}')
                        || code == KeyCode::Char('\u{8}'))
                {
                    delete_word(&mut modal.session_name);
                    return;
                }
                if modifiers.contains(KeyModifiers::ALT) && code == KeyCode::Backspace {
                    delete_word(&mut modal.session_name);
                    return;
                }
                if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('u') {
                    modal.session_name.clear();
                    return;
                }

                match code {
                    KeyCode::Esc => {
                        self.modal = Modal::None;
                    }
                    KeyCode::Enter => {
                        self.confirm_new_session();
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        modal.focus = NewSessionFocus::FolderSelection;
                    }
                    KeyCode::Backspace => {
                        modal.session_name.pop();
                    }
                    KeyCode::Char(c) => {
                        modal.session_name.push(c);
                    }
                    _ => {}
                }
            }
            NewSessionFocus::FolderSelection => {
                match code {
                    KeyCode::Esc => {
                        modal.focus = NewSessionFocus::SessionName;
                    }
                    KeyCode::Enter => {
                        if modal.selected_option == 3 {
                            modal.focus = NewSessionFocus::CustomPathInput;
                        } else {
                            self.confirm_new_session();
                        }
                    }
                    KeyCode::Char('1') => modal.selected_option = 0,
                    KeyCode::Char('2') => modal.selected_option = 1,
                    KeyCode::Char('3') => modal.selected_option = 2,
                    KeyCode::Char('4') => {
                        modal.selected_option = 3;
                        modal.focus = NewSessionFocus::CustomPathInput;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        modal.selected_option = (modal.selected_option + 1).min(3);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if modal.selected_option == 0 {
                            modal.focus = NewSessionFocus::SessionName;
                        } else {
                            modal.selected_option = modal.selected_option - 1;
                        }
                    }
                    KeyCode::BackTab => {
                        modal.focus = NewSessionFocus::SessionName;
                    }
                    KeyCode::Tab => {
                        if modal.selected_option == 3 {
                            modal.focus = NewSessionFocus::CustomPathInput;
                        } else {
                            modal.selected_option = (modal.selected_option + 1) % 4;
                        }
                    }
                    KeyCode::Char('g') | KeyCode::Char(' ') => {
                        modal.init_git = !modal.init_git;
                    }
                    _ => {}
                }
            }
            NewSessionFocus::CustomPathInput => {
                if modifiers.contains(KeyModifiers::CONTROL)
                    && (code == KeyCode::Char('w')
                        || code == KeyCode::Char('h')
                        || code == KeyCode::Backspace
                        || code == KeyCode::Char('\u{17}')
                        || code == KeyCode::Char('\u{8}'))
                {
                    delete_word(&mut modal.custom_path);
                    return;
                }
                if modifiers.contains(KeyModifiers::ALT) && code == KeyCode::Backspace {
                    delete_word(&mut modal.custom_path);
                    return;
                }
                if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('u') {
                    modal.custom_path.clear();
                    return;
                }

                match code {
                    KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
                        modal.focus = NewSessionFocus::FolderSelection;
                    }
                    KeyCode::Enter => {
                        self.confirm_new_session();
                    }
                    KeyCode::Backspace => {
                        modal.custom_path.pop();
                    }
                    KeyCode::Char(c) => {
                        modal.custom_path.push(c);
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_confirm_kill_modal_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_kill();
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.modal = Modal::None;
            }
            _ => {}
        }
    }
}

fn get_git_branch(path: &Path) -> Option<String> {
    let git_head = path.join(".git/HEAD");
    if git_head.exists() {
        if let Ok(content) = std::fs::read_to_string(&git_head) {
            let trimmed = content.trim();
            if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                return Some(branch.to_string());
            }
            if trimmed.len() >= 7 {
                return Some(trimmed[..7].to_string());
            }
        }
    }

    // Worktrees have .git as a file pointing to gitdir: ...
    let git_file = path.join(".git");
    if git_file.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_file) {
            if let Some(gitdir_str) = content.trim().strip_prefix("gitdir: ") {
                let gitdir = PathBuf::from(gitdir_str);
                let head = gitdir.join("HEAD");
                if let Ok(head_content) = std::fs::read_to_string(&head) {
                    let trimmed = head_content.trim();
                    if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                        return Some(branch.to_string());
                    }
                }
            }
        }
    }

    None
}

fn get_git_last_commit(path: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%h - %s (%cr)"])
        .current_dir(path)
        .output()
        .ok()?;

    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

pub fn delete_word(s: &mut String) {
    while let Some(c) = s.chars().last() {
        if c.is_whitespace() || c == '/' || c == '-' || c == '_' || c == '.' {
            s.pop();
        } else {
            break;
        }
    }
    while let Some(c) = s.chars().last() {
        if c.is_whitespace() || c == '/' || c == '-' || c == '_' || c == '.' {
            break;
        }
        s.pop();
    }
}
