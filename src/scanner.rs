use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::items::{Candidate, ItemKind};
use crate::tmux;

type WorktreeCache = HashMap<PathBuf, (Instant, Vec<PathBuf>)>;

pub struct BackgroundScanner {
    requests: std::sync::mpsc::Sender<Config>,
    results: std::sync::mpsc::Receiver<Vec<Candidate>>,
}

impl BackgroundScanner {
    pub fn new() -> Self {
        let (requests, incoming) = std::sync::mpsc::channel::<Config>();
        let (outgoing, results) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut cache = WorktreeCache::new();
            while let Ok(config) = incoming.recv() {
                if outgoing.send(scan_all(&config, &mut cache)).is_err() {
                    break;
                }
            }
        });
        Self { requests, results }
    }

    pub fn request(&self, config: Config) {
        let _ = self.requests.send(config);
    }

    pub fn poll(&self) -> Option<Vec<Candidate>> {
        self.results.try_recv().ok()
    }
}

fn scan_all(config: &Config, cache: &mut WorktreeCache) -> Vec<Candidate> {
    let active_sessions = tmux::list_active_sessions();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();

    // 1. Scan configured directories
    for base in &config.paths {
        if !base.is_dir() {
            continue;
        }

        let is_config_dir = base.to_string_lossy().contains(".config");

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

            let parent_name = base.file_name().and_then(|n| n.to_str()).unwrap_or("");

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

            if !seen_paths.insert(fs::canonicalize(&path).unwrap_or_else(|_| path.clone())) {
                continue;
            }
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

    // Ask Git for registered worktrees, independent of their directory layout.
    if config.scan_worktrees {
        let repos: Vec<PathBuf> = candidates
            .iter()
            .filter(|c| {
                c.path.join(".git").exists()
                    || c.path.join("HEAD").is_file() && c.path.join("objects").is_dir()
            })
            .map(|c| c.path.clone())
            .collect();
        cache.retain(|path, _| repos.contains(path));
        let mut discovered = Vec::new();
        let mut pending = Vec::new();
        for repo in repos {
            if let Some((time, paths)) = cache.get(&repo) {
                if time.elapsed() < Duration::from_secs(10) {
                    discovered.push((repo, paths.clone()));
                    continue;
                }
            }
            pending.push(repo);
        }
        let next = AtomicUsize::new(0);
        let results = Mutex::new(Vec::new());
        // Bound process creation even when the project list is large.
        std::thread::scope(|scope| {
            for _ in 0..pending.len().min(4) {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(repo) = pending.get(index) else {
                            break;
                        };
                        let paths = list_worktrees(repo);
                        results.lock().unwrap().push((repo.clone(), paths));
                    }
                });
            }
        });
        for (repo, paths) in results.into_inner().unwrap() {
            if let Some(paths) = paths {
                cache.insert(repo.clone(), (Instant::now(), paths.clone()));
                discovered.push((repo, paths));
            }
        }
        discovered.sort_by(|a, b| {
            b.0.join(".git")
                .is_dir()
                .cmp(&a.0.join(".git").is_dir())
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut seen_worktrees = HashSet::new();
        for (repo, paths) in discovered {
            let repo_name = repo.file_name().unwrap_or_default().to_string_lossy();
            for path in paths {
                if !path.is_dir() {
                    continue;
                }
                let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if !seen_worktrees.insert(identity) {
                    continue;
                }
                let wt_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let candidate = Candidate {
                    display_name: format!("{} [wt: {}]", repo_name, wt_name),
                    session_name: format!("wt_{}__{}", repo_name, wt_name),
                    path,
                    kind: ItemKind::Worktree {
                        parent_repo: repo_name.to_string(),
                        wt_name,
                    },
                    is_running: false,
                    is_attached: false,
                    windows_count: None,
                };
                if let Some(existing) = candidates
                    .iter_mut()
                    .find(|c| paths_match(&c.path, &candidate.path))
                {
                    *existing = candidate;
                } else {
                    candidates.push(candidate);
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

    // 5. Sort: running sessions first, then projects over configs, then alphabetical by display name
    candidates.sort_by(|a, b| match (a.is_running, b.is_running) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.kind.priority().cmp(&b.kind.priority()).then_with(|| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
        }),
    });

    candidates
}

fn list_worktrees(repo: &Path) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_worktrees(&output.stdout))
}

fn parse_worktrees(output: &[u8]) -> Vec<PathBuf> {
    output
        .split(|b| *b == 0)
        .filter_map(|field| field.strip_prefix(b"worktree "))
        // Git lists the main checkout (or bare repository) first.
        .skip(1)
        .map(|path| {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                PathBuf::from(std::ffi::OsStr::from_bytes(path))
            }
            #[cfg(not(unix))]
            {
                PathBuf::from(String::from_utf8_lossy(path).into_owned())
            }
        })
        .collect()
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
    fn parses_nul_delimited_paths_and_excludes_main_checkout() {
        let output = b"worktree /repo\0HEAD abc\0branch refs/heads/main\0\0worktree /outside/a b\nfeature\0HEAD abc\0detached\0\0";
        assert_eq!(
            parse_worktrees(output),
            vec![PathBuf::from("/outside/a b\nfeature")]
        );
        assert!(parse_worktrees(b"").is_empty());
    }

    #[test]
    fn discovers_registered_worktrees_outside_conventional_folder() {
        let root = std::env::temp_dir().join(format!(
            "sessionizer-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let projects = root.join("projects");
        let repo = projects.join("repo");
        let external = root.join("elsewhere/feature with spaces");
        fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ]);
        git(&["worktree", "add", "--detach", external.to_str().unwrap()]);
        let sibling = projects.join("a-sibling");
        git(&["worktree", "add", "--detach", sibling.to_str().unwrap()]);
        fs::create_dir_all(repo.join(".worktrees/not-registered")).unwrap();
        let config = Config {
            paths: vec![projects.clone(), projects],
            ..Config::default()
        };
        let mut cache = WorktreeCache::new();
        let candidates = scan_all(&config, &mut cache);
        let worktrees: Vec<_> = candidates.iter().filter(|c| c.is_worktree()).collect();
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees.iter().any(|c| c.path == external));
        assert!(worktrees.iter().any(|c| c.path == sibling));
        assert!(worktrees.iter().all(
            |c| matches!(&c.kind, ItemKind::Worktree { parent_repo, .. } if parent_repo == "repo")
        ));
        assert_eq!(cache.len(), 2);
        // Missing worktrees are filtered even while their Git records are cached.
        fs::remove_dir_all(&external).unwrap();
        assert!(
            !scan_all(&config, &mut cache)
                .iter()
                .any(|c| c.path == external)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_scanner_discovers_items() {
        let config = load_config();
        let candidates = scan_all(&config, &mut WorktreeCache::new());
        assert!(!candidates.is_empty(), "Should discover candidates");
        println!("Discovered {} candidates:", candidates.len());
        for c in &candidates {
            println!(
                "  [{}] {} -> session: {} (running: {})",
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

    #[test]
    fn test_project_priority_over_configs() {
        let mut candidates = vec![
            Candidate {
                display_name: ".config/nvim".into(),
                session_name: "cfg_nvim".into(),
                path: PathBuf::from("/home/user/.config/nvim"),
                kind: ItemKind::Config,
                is_running: false,
                is_attached: false,
                windows_count: None,
            },
            Candidate {
                display_name: "Projects/sessionizer".into(),
                session_name: "sessionizer".into(),
                path: PathBuf::from("/home/user/Projects/sessionizer"),
                kind: ItemKind::Project,
                is_running: false,
                is_attached: false,
                windows_count: None,
            },
            Candidate {
                display_name: ".config/alacritty".into(),
                session_name: "cfg_alacritty".into(),
                path: PathBuf::from("/home/user/.config/alacritty"),
                kind: ItemKind::Config,
                is_running: true,
                is_attached: false,
                windows_count: None,
            },
            Candidate {
                display_name: "Projects/my-app".into(),
                session_name: "my-app".into(),
                path: PathBuf::from("/home/user/Projects/my-app"),
                kind: ItemKind::Project,
                is_running: true,
                is_attached: false,
                windows_count: None,
            },
        ];

        candidates.sort_by(|a, b| match (a.is_running, b.is_running) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.kind.priority().cmp(&b.kind.priority()).then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            }),
        });

        assert_eq!(candidates[0].display_name, "Projects/my-app");
        assert_eq!(candidates[1].display_name, ".config/alacritty");
        assert_eq!(candidates[2].display_name, "Projects/sessionizer");
        assert_eq!(candidates[3].display_name, ".config/nvim");
    }
}
