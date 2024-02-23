use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("I/O error at path {}: {}", path.to_string_lossy(), message)]
    IOError {
        #[source]
        source: std::io::Error,
        path: PathBuf,
        message: &'static str,
    },

    #[error("Failed to parse JSON")]
    JsonParseError(serde_json::Error),
}
