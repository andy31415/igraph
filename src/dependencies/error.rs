use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("I/O error: {}", message)]
    IOError {
        #[source]
        source: std::io::Error,
        message: &'static str,
    },

    #[error("I/O error at path {}: {}", path.to_string_lossy(), message)]
    FileIOError {
        #[source]
        source: std::io::Error,
        path: PathBuf,
        message: &'static str,
    },

    #[error("Terra render error")]
    RenderError(tera::Error),

    #[error("Failed to parse JSON")]
    JsonParseError(serde_json::Error),

    // std::thread panics do not return printable values by default, and while
    // it might be possible to upcast to a `dyn Debug`, it is hardly worth it
    // here.
    #[error("Subtask join error")]
    JoinError,

    #[error("Internal error")]
    Internal { message: String },

    #[error("Error parsing a config file (nom error)")]
    ConfigParseError { message: String },
}
