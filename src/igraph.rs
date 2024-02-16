use std::{
    fs::File,
    io::{BufRead, BufReader, Read as _},
    path::{Path, PathBuf},
};

use regex::Regex;
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

#[derive(Debug, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SourceFileEntry {
    pub file_path: PathBuf,
    pub include_directories: Vec<PathBuf>,
}

#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
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
            path: file_path.clone(),
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

#[cfg(feature = "ssr")]
/// Attempt to make the full path of head::tail
/// returns None if that fails (e.g. path does not exist)
fn try_resolve(head: &Path, tail: &PathBuf) -> Option<PathBuf> {
    head.join(tail).canonicalize().ok()
}

#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
pub fn extract_includes(path: &PathBuf, include_dirs: &[PathBuf]) -> Result<Vec<PathBuf>, Error> {
    use tracing::debug;

    let f = File::open(path).map_err(|source| Error::IOError {
        source,
        path: path.clone(),
        message: "open",
    })?;

    let reader = BufReader::new(f);

    let inc_re = Regex::new(r##"^\s*#include\s*(["<])([^">]*)[">]"##).unwrap();

    let mut result = Vec::new();
    let parent_dir = PathBuf::from(path.parent().unwrap());

    for line in reader.lines() {
        let line = line.map_err(|source| Error::IOError {
            source,
            path: path.clone(),
            message: "line read",
        })?;

        if let Some(captures) = inc_re.captures(&line) {
            let inc_type = captures.get(1).unwrap().as_str();
            let relative_path = PathBuf::from(captures.get(2).unwrap().as_str());
            
            debug!("Possible include: {:?}", relative_path);

            if inc_type == "\"" {
                if let Some(p) = try_resolve(&parent_dir, &relative_path) {
                    result.push(p);
                    continue;
                }
            }

            if let Some(p) = include_dirs
                .iter()
                .filter_map(|i| try_resolve(i, &relative_path))
                .next()
            {
                result.push(p);
            } else {
                debug!("NOT resolved via {:?}", include_dirs);
            }
        }
    }

    Ok(result)
}
