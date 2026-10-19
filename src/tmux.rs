use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub name: String,
    pub attached: bool,
    pub windows_count: usize,
    pub path: PathBuf,
}

pub fn list_active_sessions() -> Vec<ActiveSession> {
    let output = match Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}\t#{session_attached}\t#{session_windows}\t#{session_path}"])
        .output()
    {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.is_empty() {
                return None;
            }
            let name = parts[0].to_string();
            let attached = parts.get(1).map_or(false, |s| s.trim() != "0");
            let windows_count = parts.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(1);
            let path = parts.get(3).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));

            Some(ActiveSession {
                name,
                attached,
                windows_count,
                path,
            })
        })
        .collect()
}

pub fn list_windows(session_name: &str) -> Vec<String> {
    let output = match Command::new("tmux")
        .args([
            "list-windows",
            "-t",
            session_name,
            "-F",
            "#{window_index}: #{window_name} #{?window_active,● ,}#{?pane_current_command,[#{pane_current_command}],}",
        ])
        .output()
    {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect()
}

pub fn kill_session(session_name: &str) -> bool {
    Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn session_exists(session_name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", session_name])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

pub fn launch_session(
    session_name: &str,
    project_path: &Path,
    is_worktree: bool,
    wt_name: Option<&str>,
    editor: &str,
) {
    let exists = session_exists(session_name);

    if !exists {
        let path_str = project_path.to_string_lossy();

        // 1. Create new detached session with term1
        let _ = Command::new("tmux")
            .args(["new-session", "-d", "-s", session_name, "-n", "term1", "-c", &path_str])
            .status();

        // 2. Editor window
        let _ = Command::new("tmux")
            .args([
                "new-window",
                "-t",
                &format!("{}:2", session_name),
                "-n",
                "editor",
                "-c",
                &path_str,
                editor,
            ])
            .status();

        // 3. Term2 window
        let _ = Command::new("tmux")
            .args([
                "new-window",
                "-t",
                &format!("{}:3", session_name),
                "-n",
                "term2",
                "-c",
                &path_str,
            ])
            .status();

        // 4. Select editor window
        let _ = Command::new("tmux")
            .args(["select-window", "-t", &format!("{}:2", session_name)])
            .status();

        // 5. Worktree style
        if is_worktree {
            let _ = Command::new("tmux")
                .args([
                    "set-option",
                    "-t",
                    session_name,
                    "status-style",
                    "bg=#d75f00,fg=#000000,bold",
                ])
                .status();

            if let Some(wt) = wt_name {
                let _ = Command::new("tmux")
                    .args([
                        "set-option",
                        "-t",
                        session_name,
                        "status-left",
                        &format!("  [WORKTREE: {}] ", wt),
                    ])
                    .status();
            }
        }
    }

    // Attach or switch client
    if std::env::var("TMUX").is_ok() {
        let _ = Command::new("tmux")
            .args(["switch-client", "-t", session_name])
            .status();
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let err = Command::new("tmux")
                .args(["attach", "-t", session_name])
                .exec();
            eprintln!("Failed to exec tmux attach: {}", err);
        }
        #[cfg(not(unix))]
        {
            let _ = Command::new("tmux")
                .args(["attach", "-t", session_name])
                .status();
        }
    }
}
