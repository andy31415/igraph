use igraph::igraph::{
    compiledb::parse_compile_database,
    cparse::{all_sources_and_includes, SourceWithIncludes},
    path_mapper::{PathMapper, PathMapping},
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use tracing::{error, info, trace};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[derive(Debug, PartialEq, Clone)]
struct Mapping {
    path: String,
    mapped: Option<String>,
}

impl Mapping {
    pub fn of(path: &Path, mapper: &PathMapper) -> Self {
        Self {
            path: path.to_string_lossy().into(),
            mapped: mapper.try_map(path),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
struct IncludeInfo {
    file: Mapping,
    includes: Vec<Mapping>,
}

impl IncludeInfo {
    pub fn of(data: &SourceWithIncludes, mapping: &PathMapper) -> Self {
        Self {
            file: Mapping::of(&data.path, mapping),
            includes: data
                .includes
                .iter()
                .map(|p| Mapping::of(p, mapping))
                .collect(),
        }
    }
}

#[tokio::main]
async fn main() {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();

    let mut mapper = PathMapper::default();

    mapper.add_mapping(PathMapping {
        from: PathBuf::from("/home/andrei/devel/connectedhomeip/src/app"),
        to: "app::".into(),
    });

    let mut includes = HashSet::new();

    const COMPILE_DB_PATH: &str =
        "/home/andrei/devel/connectedhomeip/out/linux-x64-all-clusters-clang/compile_commands.json";

    info!("Loading compile db...");
    let r = parse_compile_database(COMPILE_DB_PATH).await;

    info!("Done ...");
    match r {
        Ok(data) => {
            for entry in data {
                for i in entry.include_directories {
                    includes.insert(i);
                }
            }
        }
        Err(e) => error!("ERROR: {:#?}", e),
    }

    let includes = includes.into_iter().collect::<Vec<_>>();

    info!("Processing with {} includes", includes.len());
    trace!("Processing with includes {:#?}", includes);

    let data = all_sources_and_includes(
        glob::glob("/home/andrei/devel/connectedhomeip/src/app/**/*").expect("Valid pattern"),
        &includes,
    )
    .await;

    let data = match data {
        Ok(value) => value,
        Err(e) => {
            error!("ERROR: {:#?}", e);
            return;
        }
    };

    for r in data.iter().map(|v| IncludeInfo::of(v, &mapper)) {
        trace!("GOT: {:?}", r);
    }

    info!("Done {} files", data.len());
}
