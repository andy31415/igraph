use std::{fs::File, io::Read as _, path::PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{instrument, trace};

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

#[derive(Debug, PartialEq, PartialOrd, Hash)]
pub struct SourceFileEntry {
    pub file_path: PathBuf,
    pub include_directories: Vec<PathBuf>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("I/O error at path {}: {}", path, message)]
    IOError {
        #[source]
        source: std::io::Error,
        path: String,
        message: &'static str,
    },

    #[error("Failed to parse JSON")]
    JsonParseError(serde_json::Error),
}

impl TryFrom<CompileCommandsEntry> for SourceFileEntry {
    type Error = Error;

    #[instrument]
    fn try_from(value: CompileCommandsEntry) -> Result<Self, Self::Error> {
        trace!("Converting CompileCommandsEntry to SourceFileEntry");

        let start_dir = PathBuf::from(value.directory);

        let source_file = PathBuf::from(value.file);
        let file_path = if source_file.is_relative() {
            start_dir.join(source_file)
        } else {
            source_file
        };

        let file_path = file_path.canonicalize().map_err(|source| Error::IOError {
            source,
            path: file_path.to_string_lossy().into(),
            message: "canonicalize",
        })?;

        Ok(SourceFileEntry {
            file_path,
            include_directories: vec![],
        })
    }
}

#[instrument]
pub fn parse_compile_database(path: &str) -> Result<Vec<SourceFileEntry>, Error> {
    let mut file = File::open(path).map_err(|source| Error::IOError {
        source,
        path: path.into(),
        message: "open",
    })?;
    let mut json_string = String::new();

    file.read_to_string(&mut json_string)
        .map_err(|source| Error::IOError {
            source,
            path: path.into(),
            message: "read_to_string",
        })?;

    let raw_items: Vec<CompileCommandsEntry> =
        serde_json::from_str(&json_string).map_err(Error::JsonParseError)?;

    Ok(raw_items
        .into_iter()
        .map(|x| SourceFileEntry::try_from(x))
        .filter_map(|r| r.ok())
        .collect())
}
