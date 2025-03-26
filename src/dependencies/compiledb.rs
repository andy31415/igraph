use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use tracing::debug;

use super::canonicalize::canonicalize_cached;
use super::error::Error;

#[derive(Debug, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SourceFileEntry {
    pub file_path: PathBuf,
    pub include_directories: Vec<PathBuf>,
}

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

impl TryFrom<CompileCommandsEntry> for SourceFileEntry {
    type Error = Error;

    fn try_from(value: CompileCommandsEntry) -> Result<Self, Self::Error> {
        // trace!("Generating SourceFileEntry {:#?}", value);

        let start_dir = PathBuf::from(value.directory);

        let source_file = PathBuf::from(value.file);
        let file_path = if source_file.is_relative() {
            start_dir.join(source_file)
        } else {
            source_file
        };

        let file_path = canonicalize_cached(file_path)
            .map_err(|source| Error::IOError {
                source,
                message: "canonicalize",
            })?
            .ok_or(Error::FileNotFound)?;

        let args = value
            .arguments
            .unwrap_or_else(|| shlex::split(&value.command.unwrap()).unwrap());

        let include_directories = args
            .iter()
            .filter_map(|a| a.strip_prefix("-I"))
            .map(PathBuf::from)
            .filter_map(|p| {
                if p.is_relative() {
                    canonicalize_cached(start_dir.join(p)).ok()?
                } else {
                    Some(p)
                }
            })
            .collect();

        Ok(SourceFileEntry {
            file_path,
            include_directories,
        })
    }
}

pub fn parse_compile_database(path: &str) -> Result<Vec<SourceFileEntry>, Error> {
    let mut file = File::open(path).map_err(|source| Error::FileIOError {
        source,
        path: path.into(),
        message: "open",
    })?;
    let mut json_string = String::new();

    file.read_to_string(&mut json_string)
        .map_err(|source| Error::FileIOError {
            source,
            path: path.into(),
            message: "read_to_string",
        })?;

    let raw_items: Vec<CompileCommandsEntry> =
        serde_json::from_str(&json_string).map_err(Error::JsonParseError)?;

    Ok(raw_items
        .into_iter()
        .filter(|e| {
            e.file.ends_with(".cpp")
                || e.file.ends_with(".cc")
                || e.file.ends_with(".cxx")
                || e.file.ends_with(".c")
                || e.file.ends_with(".h")
                || e.file.ends_with(".hpp")
        })
        .map(SourceFileEntry::try_from)
        .inspect(|r| {
            if let Err(e) = r {
                debug!(target: "compile-db", "Failed to parse: {:?}", e);
            }
        })
        .filter_map(|r| r.ok())
        .collect())
}
