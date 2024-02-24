use std::path::PathBuf;

use tokio::task::JoinError;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("I/O error at path {}: {}", path.to_string_lossy(), message)]
    IOError {
        #[source]
        source: std::io::Error,
        path: PathBuf,
        message: &'static str,
    },
    
    #[error("I/O error: {}", message)]
    AsyncIOError {
        #[source]
        source: tokio::io::Error,
        message: &'static str,
    },

    #[error("Terra render error")]
    RenderError(tera::Error),

    #[error("Failed to parse JSON")]
    JsonParseError(serde_json::Error),

    #[error("Subtask join error")]
    JoinError(JoinError),

    #[error("Internal error")]
    Internal { message: String },

    #[error("Error parsing a config file (nom error)")]
    ConfigParseError { message: String },
}
