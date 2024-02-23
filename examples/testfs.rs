use igraph::igraph::{
    compiledb::parse_compile_database,
    cparse::{all_sources_and_includes, SourceWithIncludes},
    gn::load_gn_targets,
    path_mapper::{PathMapper, PathMapping},
};
use nom::{
    branch::alt,
    bytes::complete::{is_not, tag},
    character::complete::{char as parsed_char, multispace1},
    combinator::{opt, value},
    multi::{many1, separated_list0},
    sequence::{pair, separated_pair, tuple},
    IResult, Parser,
};

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use tracing::{error, info, trace};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[derive(Debug)]
struct DependencyData {
    includes: Vec<PathBuf>,
}

fn expand_variable(value: &str, existing: &HashMap<String, String>) -> String {
    // expand any occurences of "${name}"
    let mut value = value.to_string();

    loop {
        let replacements = existing
            .iter()
            .map(|(k, v)| (format!("${{{}}}", k), v))
            .filter(|(k, _v)| value.contains(k))
            .collect::<Vec<_>>();

        if replacements.is_empty() {
            break;
        }

        for (k, v) in replacements {
            value = value.replace(&k, v);
        }
    }

    value
}

fn parse_comment(input: &str) -> IResult<&str, &str> {
    pair(parsed_char('#'), is_not("\n\r"))
        .map(|(_, r)| r)
        .parse(input)
}

fn parse_whitespace(input: &str) -> IResult<&str, ()> {
    value((), many1(alt((multispace1, parse_comment)))).parse(input)
}

fn parse_variable_name(input: &str) -> IResult<&str, &str> {
    is_not("= \t\r\n{}[]()#").parse(input)
}

enum InputCommand {
    IncludesFromCompileDb(String),
    IncludeDirectory(String),
    Glob(String)
}

fn parse_input(input: &str) -> IResult<&str, InputCommand> {
    tuple((
        tuple((tag("input")), parse_whitespace, tag("{"), opt(parse_whitespace)),
        separated_list0(parse_whitespace, parse_input_command),
        tuple((parse_whitespace, tag("}"), opt(parse_whitespace)),
    )).map(|_, l, _| l).parse(input)
    // Next input follows:
    //
    // input {
    //   includes from compiledb ${COMPILE_ROOT}/compile_commands.json
    //   include_dir ${GEN_ROOT}

    //   # Only API will be loaded anyway
    //   glob ${CHIP_ROOT}/src/app/**
    //   glob ${GEN_ROOT}/**
    // }
}


fn parse_data(input: &str) -> IResult<&str, ()> {
    let input = match parse_whitespace(input) {
        Ok((data, _)) => data,
        _ => input,
    };

    // First, parse all variables
    let (_input, input_vars) = separated_list0(
        parse_whitespace,
        separated_pair(parse_variable_name, tag("="), is_not("#\n\r \t")),
    )
    .parse(input)?;

    let mut variables = HashMap::new();
    for (name, value) in input_vars {
        variables.insert(name.to_string(), expand_variable(value, &variables));
    }

    trace!("Resolved variables: {:#?}", variables);

    let instructions = tuple((opt(parse_whitespace), parse_input, opt(parse_whitespace)))
        .map(|(_, i, _)| i)
        .parse(input)?;

    Ok(("", ()))
}

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

    let _ = parse_data(include_str!("../sample_api.txt"));

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

    info!("Loading GN targets");
    // validate gn
    match load_gn_targets(
        &PathBuf::from("/home/andrei/devel/connectedhomeip/out/linux-x64-all-clusters-clang"),
        &PathBuf::from("/home/andrei/devel/connectedhomeip"),
        "//src/app/*",
    )
    .await
    {
        Ok(items) => {
            for target in items.iter() {
                trace!("  {:#?}", target);
            }
            info!("Found {} gn targets", items.len());
        }
        Err(e) => {
            error!("GN LOAD ERROR: {:?}", e);
        }
    }
}
