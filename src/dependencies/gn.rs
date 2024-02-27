use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use tokio::process::Command;
use tracing::error;

use super::error::Error;

#[derive(Debug, PartialEq)]
pub struct GnTarget {
    pub name: String,
    pub sources: Vec<PathBuf>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct GnSourcesOutput {
    inner: HashMap<String, SourcesData>,
}

#[derive(Deserialize)]
struct SourcesData {
    sources: Option<Vec<String>>,
}

pub async fn load_gn_targets(
    gn_dir: &Path,
    source_root: &Path,
    target: &str,
    rundir: &Option<String>,
) -> Result<Vec<GnTarget>, Error> {
    // TODO: GN PATH?
    let mut command = Command::new("/usr/bin/gn");
    command.arg("desc");
    command.arg("--format=json");
    command.arg(format!(
        "--root={}",
        source_root
            .canonicalize()
            .map_err(|e| Error::Internal {
                message: format!("Canonical path: {:?}", e),
            })?
            .to_string_lossy(),
    ));
    command.arg(gn_dir.canonicalize().map_err(|e| Error::Internal {
        message: format!("Canonical path: {:?}", e),
    })?);
    command.arg(target);
    command.arg("sources");

    if let Some(d) = rundir {
        command.current_dir(d);
    }

    let output = command.output().await.map_err(|e| Error::Internal {
        message: format!("Canonical path: {:?}", e),
    })?;

    if !output.status.success() {
        let data = String::from_utf8_lossy(&output.stdout);
        if data.len() > 0 {
            for l in data.lines() {
                error!("STDOUT: {}", l);
            }
        }

        let data = String::from_utf8_lossy(&output.stderr);
        if data.len() > 0 {
            for l in data.lines() {
                error!("STDERR: {}", l);
            }
        }

        return Err(Error::Internal {
            message: format!("Failed to execute GN. Status {:?}.", output.status),
        });
    }

    let decoded: GnSourcesOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| Error::Internal {
            message: format!("JSON parse error: {:?}", e),
        })?;

    // filter-map because not all targets have sources. However the ones that do have
    // can be recognized
    Ok(decoded
        .inner
        .into_iter()
        .filter_map(|(name, sources)| {
            sources.sources.map(|sources| GnTarget {
                name,
                sources: sources
                    .into_iter()
                    .map(|s| {
                        let mut path = PathBuf::from(source_root);
                        // NOTE: we RELY that paths start with '//'
                        if !s.starts_with("//") {
                            error!("PATH: {:?} DOES NOT start with '//' as expected", s);
                        }
                        path.push(PathBuf::from(&s.as_str()[2..]));
                        path
                    })
                    .collect(),
            })
        })
        .filter(|t| !t.sources.is_empty())
        .collect())
}
