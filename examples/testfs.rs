use igraph::igraph::{
    compiledb::parse_compile_database,
    cparse::{extract_includes, FileType},
    path_mapper::{PathMapper, PathMapping},
};
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tracing::{error, info, trace};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[derive(Debug, PartialEq, Clone)]
struct Mapping {
    path: String,
    mapped: Option<String>,
}

#[derive(Debug, PartialEq, Clone)]
struct IncludeInfo {
    file: Mapping,
    includes: Vec<Mapping>,
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

    let mapper = Arc::new(mapper);

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

    let includes = Arc::new(includes.into_iter().collect::<Vec<_>>());

    info!("Processing with {} includes", includes.len());
    trace!("Processing with includes {:#?}", includes);

    let (tx, mut rx) = mpsc::channel(10);

    for entry in
        glob::glob("/home/andrei/devel/connectedhomeip/src/app/**/*").expect("Valid pattern")
    {
        let spawn_tx = tx.clone();
        let mapper = mapper.clone();
        let includes = includes.clone();
        tokio::spawn(async move {
            match entry {
                Ok(s) if FileType::of(&s) != FileType::Unknown => {
                    trace!("PROCESS: {:?}", s);
                    let r = IncludeInfo {
                        file: Mapping {
                            path: s.to_string_lossy().into(),
                            mapped: mapper.try_map(&s),
                        },
                        includes: extract_includes(&s, &includes)
                            .await
                            .unwrap()
                            .into_iter()
                            .map(|v| Mapping {
                                path: v.to_string_lossy().into(),
                                mapped: mapper.try_map(&v),
                            })
                            .collect(),
                    };

                    if let Err(e) = spawn_tx.send(r).await {
                        error!("Error sending: {:?}", e);
                    }
                }
                Ok(_) => {}
                Err(e) => error!("GLOB error: {:?}", e),
            };
        });
    }
    drop(tx);

    let mut cnt = 0;
    while let Some(r) = rx.recv().await {
        trace!("GOT: {:?}", r);
        cnt += 1;
    }
    info!("Done {} files", cnt);
}
