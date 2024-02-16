use std::{collections::HashSet, path::PathBuf};
use igraph::{igraph::{extract_includes, parse_compile_database}, path_mapper::{PathMapper, PathMapping}};
use tracing::{info, info_span, warn};
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

    if let Ok(data) = parse_compile_database("/home/andrei/devel/connectedhomeip/out/linux-x64-all-clusters-clang/compile_commands.json") {
        for entry in data {
            for i in entry.include_directories {
                includes.insert(i);
            }
        }
    }

    let includes = includes.into_iter().collect::<Vec<_>>();
    
    let span = info_span!("processing");
    for entry in glob::glob("/home/andrei/devel/connectedhomeip/src/app/**/*").expect("Valid pattern") {
        let _enter = span.enter();
        match entry {
            Ok(s) if is_header(&s) || is_source(&s) => {
                info!("PROCESS: {:?}", mapper.try_map(&s));
                for v in extract_includes(&s, &includes).unwrap() {
                    if let Some(p) = mapper.try_map(&v) {
                        info!("    => {:?}", p);
                    }
                }
            },
            _ => {},
        }
    }
}