use std::path::PathBuf;
use serde::{Deserialize, Serialize};

const CONFIG_FOLDER: &str = "sessionizer";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_paths")]
    pub paths: Vec<PathBuf>,

    #[serde(default = "default_only_git")]
    pub only_git_repo: bool,

    #[serde(default = "default_scan_worktrees")]
    pub scan_worktrees: bool,

    #[serde(default = "default_new_session_base_dir")]
    pub new_session_base_dir: PathBuf,

    #[serde(default = "default_editor")]
    pub editor: String,
}

fn default_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let projects = home.join("Projects");
        if projects.is_dir() {
            paths.push(projects);
        }
        let config = home.join(".config");
        if config.is_dir() {
            paths.push(config);
        }
    }
    paths
}

fn default_only_git() -> bool {
    false
}

fn default_scan_worktrees() -> bool {
    true
}

fn default_new_session_base_dir() -> PathBuf {
    dirs::home_dir().map(|h| h.join("Projects")).unwrap_or_else(|| PathBuf::from("."))
}

fn default_editor() -> String {
    std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            paths: default_paths(),
            only_git_repo: default_only_git(),
            scan_worktrees: default_scan_worktrees(),
            new_session_base_dir: default_new_session_base_dir(),
            editor: default_editor(),
        }
    }
}

pub fn expand_tilde(path: &PathBuf) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    path.clone()
}

pub fn load_config() -> Config {
    let Some(config_folder) = dirs::config_dir() else {
        return Config::default();
    };

    let sessionizer_folder = config_folder.join(CONFIG_FOLDER);
    let config_file = sessionizer_folder.join(CONFIG_FILE);

    if !config_file.exists() {
        let default_cfg = Config::default();
        if let Ok(_) = std::fs::create_dir_all(&sessionizer_folder) {
            if let Ok(toml_str) = toml::to_string_pretty(&default_cfg) {
                let _ = std::fs::write(&config_file, toml_str);
            }
        }
        return default_cfg;
    }

    match std::fs::read_to_string(&config_file) {
        Ok(content) => match toml::from_str::<Config>(&content) {
            Ok(mut cfg) => {
                cfg.paths = cfg.paths.into_iter().map(|p| expand_tilde(&p)).collect();
                cfg.new_session_base_dir = expand_tilde(&cfg.new_session_base_dir);
                cfg
            }
            Err(e) => {
                eprintln!("Error parsing {}: {}, using defaults", config_file.display(), e);
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}
