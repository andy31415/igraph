use std::{
    fs::File,
    io::{BufRead, BufReader, Read as _},
    path::PathBuf,
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument, trace};

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

    #[instrument(skip(value))]
    fn try_from(value: CompileCommandsEntry) -> Result<Self, Self::Error> {
        trace!("Generating SourceFileEntry {:#?}", value);

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

        let args = value
            .arguments
            .unwrap_or_else(|| shlex::split(&value.command.unwrap()).unwrap());

        let include_directories = args
            .iter()
            .filter_map(|a| a.strip_prefix("-I"))
            .map(PathBuf::from)
            .filter_map(|p| {
                if p.is_relative() {
                    start_dir.join(p).canonicalize().ok()
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

pub fn parse_includes(
    _path: PathBuf,
    _top_dir: PathBuf,
    _include_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    todo!()
}

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
        .filter(|e| {
            e.file.ends_with(".cpp")
                || e.file.ends_with(".cc")
                || e.file.ends_with(".cxx")
                || e.file.ends_with(".c")
                || e.file.ends_with(".h")
                || e.file.ends_with(".hpp")
        })
        .map(SourceFileEntry::try_from)
        .filter_map(|r| r.ok())
        .collect())
}

pub fn extract_includes(path: &PathBuf, include_dirs: &Vec<PathBuf>) -> Result<Vec<PathBuf>, Error> {
    let f = File::open(path).map_err(|source| Error::IOError {
        source,
        path: path.to_string_lossy().into(),
        message: "open",
    })?;

    let reader = BufReader::new(f);
    
    let inc_re = Regex::new(r##"^\s*#include\s*(["<])([^">]*)[">]"##).unwrap();

    for line in reader.lines() {
        let line = line.map_err(|source| Error::IOError {
            source,
            path: path.to_string_lossy().into(),
            message: "line read",
        })?;

        if let Some(captures) = inc_re.captures(&line) {
            let inc_type = captures.get(1).unwrap().as_str();
            info!("include: {} as {}", captures.get(2).unwrap().as_str(), inc_type);
        }
    }
    
    // TODO
    Ok(vec![])
}
