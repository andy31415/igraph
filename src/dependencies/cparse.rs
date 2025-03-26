use super::canonicalize::canonicalize_cached;
use super::error::Error;

use regex::Regex;
use std::{
    fmt::Debug,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    thread,
};
use tracing::{error, info, trace};

/// Attempt to make the full path of head::tail
/// returns None if that fails (e.g. path does not exist)
fn try_resolve(head: &Path, tail: &Path) -> Option<PathBuf> {
    canonicalize_cached(head.join(tail)).ok()?
}

#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum FileType {
    Header,
    Source,
    Unknown,
}

impl FileType {
    pub fn of(path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "h" | "hpp" => FileType::Header,
            "c" | "cpp" | "cc" | "cxx" => FileType::Source,
            _ => FileType::Unknown,
        }
    }
}

static INCLUDE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r##"^\s*#include\s*(["<])([^">]*)[">]"##).unwrap());

/// Given a C-like source, try to resolve includes.
///
/// Includes are generally of the form `#include <name>` or `#include "name"`
pub fn extract_includes(path: &PathBuf, include_dirs: &[PathBuf]) -> Result<Vec<PathBuf>, Error> {
    let f = File::open(path).map_err(|source| Error::FileIOError {
        source,
        path: path.clone(),
        message: "open",
    })?;

    let reader = BufReader::new(f);
    let mut result = Vec::new();
    let parent_dir = PathBuf::from(path.parent().unwrap());

    let lines = reader.lines();

    for line in lines {
        let line = line.map_err(|source| Error::FileIOError {
            source,
            path: path.clone(),
            message: "line read",
        })?;

        if let Some(captures) = INCLUDE_REGEX.captures(&line) {
            let inc_type = captures.get(1).unwrap().as_str();
            let relative_path = PathBuf::from(captures.get(2).unwrap().as_str());

            trace!("Possible include: {:?}", relative_path);

            if inc_type == "\"" {
                if let Some(p) = try_resolve(&parent_dir, &relative_path) {
                    result.push(p);
                    continue;
                }
            }

            if let Some(p) = include_dirs
                .iter()
                .find_map(|i| try_resolve(i, &relative_path))
            {
                result.push(p);
            } else {
                // Debug only as this is VERY common due to C++ and system inclues,
                // like "list", "vector", "string" or even platform specific like "jni.h"
                // or non-enabled things (like openthread on a non-thread platform)
                trace!("Include {:?} could not be resolved", relative_path);
            }
        }
    }

    info!(target: "include-extract",
          "Includes for:\n  {:?}: {:#?}", path, result);

    Ok(result)
}

#[derive(Debug, PartialEq, PartialOrd)]
pub struct SourceWithIncludes {
    pub path: PathBuf,
    pub includes: Vec<PathBuf>,
}

/// Given a list of paths, figure out their dependencies
pub fn all_sources_and_includes<I, E>(
    paths: I,
    includes: &[PathBuf],
) -> Result<Vec<SourceWithIncludes>, Error>
where
    I: Iterator<Item = Result<PathBuf, E>>,
    E: Debug,
{
    let includes = Arc::new(Vec::from(includes));
    let mut handles = Vec::new();

    for entry in paths {
        let path = match entry {
            Ok(value) => canonicalize_cached(value)
                .map_err(|e| Error::Internal {
                    message: format!("{:?}", e),
                })?
                .ok_or(Error::FileNotFound)?,
            Err(e) => {
                return Err(Error::Internal {
                    message: format!("{:?}", e),
                })
            }
        };

        if FileType::of(&path) == FileType::Unknown {
            trace!("Skipping non-source: {:?}", path);
            continue;
        }

        // prepare data to move into sub-task
        let includes = includes.clone();

        handles.push(thread::spawn(move || {
            trace!("PROCESS: {:?}", path);
            let includes = match extract_includes(&path, &includes) {
                Ok(value) => value,
                Err(e) => {
                    error!("Error extracting includes: {:?}", e);
                    return Err(e);
                }
            };

            Ok(SourceWithIncludes { path, includes })
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        let res = handle.join().map_err(|_| Error::JoinError)?;
        results.push(res?);
    }

    Ok(results)
}
