use std::{fs::File, io::Read as _};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct CompileCommandsEntry {
    /// everything relative to this directory
    pub directory: String,

    /// what file this compiles
    pub file: String,

    /// command as a string only (needs split)
    pub command: Option<String>,

    /// split-out arguments for compilation
    pub arguments: Option<Vec<String>>,

    /// Optional what gets outputted
    pub output: Option<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum CompileDatabaseParseError {
    #[error("failed to open file")]
    OpenFileError(std::io::Error),

    #[error("failed to read the file")]
    ReadFileError(std::io::Error),

    #[error("Failed to parse JSON")]
    JsonParseError(serde_json::Error),
}

pub fn ParseCompileDatabase(
    path: &str,
) -> Result<Vec<CompileCommandsEntry>, CompileDatabaseParseError> {
    let mut file =
        File::open(path).map_err(CompileDatabaseParseError::OpenFileError)?;
    let mut json_string = String::new();

    file.read_to_string(&mut json_string)
        .map_err(CompileDatabaseParseError::ReadFileError)?;

    Ok(serde_json::from_str(&json_string).map_err(CompileDatabaseParseError::JsonParseError)?)
}
