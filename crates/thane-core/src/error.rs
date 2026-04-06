use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("workspace not found: {0}")]
    WorkspaceNotFound(Uuid),

    #[error("panel not found: {0}")]
    PanelNotFound(Uuid),

    #[error("pane not found: {0}")]
    PaneNotFound(Uuid),

    #[error("split tree is empty")]
    EmptySplitTree,

    #[error("cannot split: pane {0} not found in tree")]
    CannotSplit(Uuid),

    #[error("cannot close last pane")]
    CannotCloseLastPane,

    #[error("config parse error: {0}")]
    ConfigParse(String),

    #[error("config file error: {0}")]
    ConfigFile(#[from] std::io::Error),

    #[error("invalid keybinding: {0}")]
    InvalidKeybinding(String),

    #[error("{0}")]
    Generic(String),
}
