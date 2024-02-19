use igraph::{
    igraph::{extract_includes, parse_compile_database},
    path_mapper::{PathMapper, PathMapping},
};
use std::{collections::HashSet, path::PathBuf};
use tracing::{error, info, info_span, trace, warn};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

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

    let span = info_span!("processing");
    let mut cnt = 0;
    for entry in
        glob::glob("/home/andrei/devel/connectedhomeip/src/app/**/*").expect("Valid pattern")
    {
        let _enter = span.enter();
        match entry {
            Ok(s) if is_header(&s) || is_source(&s) => {
                cnt += 1;
                trace!("PROCESS: {:?}", mapper.try_map(&s));
                for v in extract_includes(&s, &includes).unwrap() {
                    if let Some(p) = mapper.try_map(&v) {
                        trace!("    => {:?}", p);
                    } else {
                        trace!("    => ???: {:?}", v);
                    }
                }
            }
            _ => {}
        }
    }
    info!("Done processing {} files", cnt);
}
