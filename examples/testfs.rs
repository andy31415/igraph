use igraph::{
    igraph::{extract_includes, parse_compile_database},
    path_mapper::{PathMapper, PathMapping},
};
use std::{collections::HashSet, path::PathBuf};
use tokio::sync::mpsc;
use tracing::{error, info, info_span, trace, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[derive(Debug, PartialEq, Clone)]
struct IncludeInfo {
    file: String,
    mapped_includes: Vec<String>,
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

    let is_header = |p: &std::path::Path| {
        let e = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        e == "h" || e == "hpp"
    };
    let is_source = |p: &std::path::Path| {
        let e = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        e == "cpp" || e == "cc" || e == "c" || e == "cxx"
    };

    let mut includes = HashSet::new();

    const COMPILE_DB_PATH: &str =
        "/home/andrei/devel/connectedhomeip/out/linux-x64-all-clusters-clang/compile_commands.json";

    info!("Loading compile db...");
    let r = parse_compile_database(COMPILE_DB_PATH);

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

    let (tx, mut rx) = mpsc::channel(10);

    for entry in
        glob::glob("/home/andrei/devel/connectedhomeip/src/app/**/*").expect("Valid pattern")
    {
        let spawn_tx = tx.clone();
        let spawn_mapper = mapper.clone();
        let spawn_includes = includes.clone();
        tokio::spawn(async move {
            match entry {
                Ok(s) if is_header(&s) || is_source(&s) => {
                    let mut r = IncludeInfo {
                        file: s.to_string_lossy().into(),
                        mapped_includes: vec![],
                    };

                    trace!("PROCESS: {:?}", spawn_mapper.try_map(&s));

                    for v in extract_includes(&s, &spawn_includes).unwrap() {
                        if let Some(p) = spawn_mapper.try_map(&v) {
                            trace!("    => {:?}", p);
                            r.mapped_includes.push(p);
                        } else {
                            trace!("    => ???: {:?}", v);
                        }
                    }
                    if let Err(e) = spawn_tx.send(r).await {
                        error!("Error sending: {:?}", e);
                    }
                }
                Ok(_) => {},
                Err(e) => error!("GLOB error: {:?}", e),
            };
        });
    }
    drop(tx);
    
    while let Some(r) = rx.recv().await {
        trace!("GOT: {:?}", r);
    }
    info!("Done");
}
