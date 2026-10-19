use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::items::{Candidate, ItemKind};
use crate::tmux;

pub fn scan_all(config: &Config) -> Vec<Candidate> {
    let active_sessions = tmux::list_active_sessions();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();

    // 1. Scan configured directories
    for base in &config.paths {
        if !base.is_dir() {
            continue;
        }

        let is_config_dir = base
            .to_string_lossy()
            .contains(".config");

        let Ok(entries) = fs::read_dir(base) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Skip hidden directories (except .config itself)
            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            if dir_name.starts_with('.') {
                continue;
            }

            if !is_config_dir && config.only_git_repo && !path.join(".git").exists() {
                continue;
            }

            let parent_name = base
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            let (display_name, session_name, kind) = if is_config_dir {
                (
                    format!(".config/{}", dir_name),
                    format!("cfg_{}", dir_name.replace('.', "_")),
                    ItemKind::Config,
                )
            } else {
                (
                    format!("{}/{}", parent_name, dir_name),
                    dir_name.replace('.', "_"),
                    ItemKind::Project,
                )
            };

            seen_paths.insert(path.clone());
            candidates.push(Candidate {
                display_name,
                session_name,
                path,
                kind,
                is_running: false,
                is_attached: false,
                windows_count: None,
            });
        }
    }

    // 2. Scan git worktrees if enabled
    if config.scan_worktrees {
        for base in &config.paths {
            if !base.is_dir() {
                continue;
            }

            let Ok(repo_entries) = fs::read_dir(base) else {
                continue;
            };

            for repo_entry in repo_entries.flatten() {
                let repo_path = repo_entry.path();
                let worktrees_dir = repo_path.join(".worktrees");
                if !worktrees_dir.is_dir() {
                    continue;
                }

                let repo_name = match repo_path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };

                let Ok(wt_entries) = fs::read_dir(&worktrees_dir) else {
                    continue;
                };

                for wt_entry in wt_entries.flatten() {
                    let wt_path = wt_entry.path();
                    if !wt_path.is_dir() {
                        continue;
                    }

                    let wt_name = match wt_path.file_name().and_then(|n| n.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };

                    let display_name = format!("Projects/{} [wt: {}]", repo_name, wt_name);
                    let session_name = format!("wt_{}__{}", repo_name, wt_name);

                    seen_paths.insert(wt_path.clone());
                    candidates.push(Candidate {
                        display_name,
                        session_name,
                        path: wt_path,
                        kind: ItemKind::Worktree {
                            parent_repo: repo_name.clone(),
                            wt_name,
                        },
                        is_running: false,
                        is_attached: false,
                        windows_count: None,
                    });
                }
            }
        }
    }

    // 3. Reconcile with active tmux sessions
    let mut matched_active_names = HashSet::new();

    for candidate in &mut candidates {
        if let Some(active) = active_sessions
            .iter()
            .find(|s| s.name == candidate.session_name || paths_match(&s.path, &candidate.path))
        {
            candidate.is_running = true;
            candidate.is_attached = active.attached;
            candidate.windows_count = Some(active.windows_count);
            matched_active_names.insert(active.name.clone());
        }
    }

    // 4. Add any running tmux session that didn't match a scanned project
    for active in &active_sessions {
        if !matched_active_names.contains(&active.name) {
            candidates.push(Candidate {
                display_name: format!("tmux: {}", active.name),
                session_name: active.name.clone(),
                path: active.path.clone(),
                kind: ItemKind::ActiveSession,
                is_running: true,
                is_attached: active.attached,
                windows_count: Some(active.windows_count),
            });
        }
    }

    // 5. Sort: running sessions first, then alphabetical by display name
    candidates.sort_by(|a, b| {
        match (a.is_running, b.is_running) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()),
        }
    });

    candidates
}

fn paths_match(p1: &Path, p2: &Path) -> bool {
    if p1 == p2 {
        return true;
    }
    match (fs::canonicalize(p1), fs::canonicalize(p2)) {
        (Ok(c1), Ok(c2)) => c1 == c2,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;

    #[test]
    fn test_scanner_discovers_items() {
        let config = load_config();
        let candidates = scan_all(&config);
        assert!(!candidates.is_empty(), "Should discover candidates");
        println!("Discovered {} candidates:", candidates.len());
        for c in &candidates {
            println!("  [{}] {} -> session: {} (running: {})", 
                match c.kind {
                    ItemKind::Worktree { .. } => "WT",
                    ItemKind::Config => "CFG",
                    ItemKind::Project => "PROJ",
                    ItemKind::ActiveSession => "TMUX",
                },
                c.display_name,
                c.session_name,
                c.is_running
            );
        }
    }
}

