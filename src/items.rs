use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    ActiveSession,
    Worktree { parent_repo: String, wt_name: String },
    Project,
    Config,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub display_name: String,
    pub session_name: String,
    pub path: PathBuf,
    pub kind: ItemKind,
    pub is_running: bool,
    pub is_attached: bool,
    pub windows_count: Option<usize>,
}

impl Candidate {
    pub fn is_worktree(&self) -> bool {
        matches!(self.kind, ItemKind::Worktree { .. })
    }

    pub fn wt_name(&self) -> Option<&str> {
        match &self.kind {
            ItemKind::Worktree { wt_name, .. } => Some(wt_name),
            _ => None,
        }
    }
}
