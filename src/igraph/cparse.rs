use super::error::Error;

use regex::Regex;
use std::path::{Path, PathBuf};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt as _, BufReader},
};
use tracing::debug;

/// Attempt to make the full path of head::tail
/// returns None if that fails (e.g. path does not exist)
fn try_resolve(head: &Path, tail: &PathBuf) -> Option<PathBuf> {
    head.join(tail).canonicalize().ok()
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

/// Given a C-like source, try to resolve includes.
///
/// Includes are generally of the form `#include <name>` or `#include "name"`
pub async fn extract_includes(
    path: &PathBuf,
    include_dirs: &[PathBuf],
) -> Result<Vec<PathBuf>, Error> {
    let f = File::open(path).await.map_err(|source| Error::IOError {
        source,
        path: path.clone(),
        message: "open",
    })?;

    let reader = BufReader::new(f);

    let inc_re = Regex::new(r##"^\s*#include\s*(["<])([^">]*)[">]"##).unwrap();

    let mut result = Vec::new();
    let parent_dir = PathBuf::from(path.parent().unwrap());

    let mut lines = reader.lines();

    loop {
        let line = lines.next_line().await.map_err(|source| Error::IOError {
            source,
            path: path.clone(),
            message: "line read",
        })?;

        let line = match line {
            Some(value) => value,
            None => break,
        };

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
                // Debug only as this is VERY common due to C++ and system inclues,
                // like "list", "vector", "string" or even platform specific like "jni.h"
                // or non-enabled things (like openthread on a non-thread platform)
                debug!("Include {:?} could not be resolved", relative_path);
            }
        }
    }

    Ok(result)
}
