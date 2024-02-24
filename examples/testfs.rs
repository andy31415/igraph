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
use nom_supreme::ParserExt;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use tracing::{debug, error, info};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[derive(Debug, Default)]
struct DependencyData {
    includes: HashSet<PathBuf>,
    files: Vec<SourceWithIncludes>,
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

fn parse_until_whitespace(input: &str) -> IResult<&str, &str> {
    is_not("#\n\r \t").parse(input)
}

#[derive(Debug)]
enum InputCommand {
    IncludesFromCompileDb(String),
    IncludeDirectory(String),
    Glob(String),
}

fn parse_input_command(input: &str) -> IResult<&str, InputCommand> {
    alt((
        parse_until_whitespace
            .preceded_by(tuple((
                tag("includes"),
                parse_whitespace,
                tag("from"),
                parse_whitespace,
                tag("compiledb"),
                parse_whitespace,
            )))
            .map(|s| InputCommand::IncludesFromCompileDb(s.into())),
        parse_until_whitespace
            .preceded_by(tuple((tag("glob"), parse_whitespace)))
            .map(|s| InputCommand::Glob(s.into())),
        parse_until_whitespace
            .preceded_by(tuple((tag("include_dir"), parse_whitespace)))
            .map(|s| InputCommand::IncludeDirectory(s.into())),
    ))
    .parse(input)
}

fn parse_input(input: &str) -> IResult<&str, Vec<InputCommand>> {
    // input {
    //   includes from compiledb ${COMPILE_ROOT}/compile_commands.json
    //   include_dir ${GEN_ROOT}

    //   # Only API will be loaded anyway
    //   glob ${CHIP_ROOT}/src/app/**
    //   glob ${GEN_ROOT}/**
    // }
    tuple((
        tuple((
            tag("input"),
            parse_whitespace,
            tag("{"),
            opt(parse_whitespace),
        )),
        separated_list0(parse_whitespace, parse_input_command),
        tuple((opt(parse_whitespace), tag("}"), opt(parse_whitespace))),
    ))
    .map(|(_, l, _)| l)
    .parse(input)
}

/// Defines an instruction regarding name mapping
#[derive(Debug)]
enum MapInstruction {
    DisplayMap { from: String, to: String },
    Keep(String),
}

#[derive(Debug)]
struct GraphInstructions {
    map_instructions: Vec<MapInstruction>,
}

impl GraphInstructions {
    fn mapped(self, variables: &HashMap<String, String>) -> Self {
        Self {
            map_instructions: self
                .map_instructions
                .into_iter()
                .map(|instruction| match instruction {
                    MapInstruction::DisplayMap { from, to } => MapInstruction::DisplayMap {
                        from: expand_variable(&from, variables),
                        to,
                    },
                    other => other,
                })
                .collect(),
        }
    }
}

fn parse_map_instructions(input: &str) -> IResult<&str, Vec<MapInstruction>> {
    separated_list0(
        parse_whitespace,
        alt((
            separated_pair(
                parse_until_whitespace,
                tuple((parse_whitespace, tag("=>"), parse_whitespace)),
                parse_until_whitespace,
            )
            .map(|(from, to)| MapInstruction::DisplayMap {
                from: from.into(),
                to: to.into(),
            }),
            parse_until_whitespace
                .preceded_by(tuple((
                    opt(parse_whitespace),
                    tag("keep"),
                    parse_whitespace,
                )))
                .map(|s| MapInstruction::Keep(s.into())),
        )),
    )
    .preceded_by(tuple((
        tag("map"),
        parse_whitespace,
        tag("{"),
        parse_whitespace,
    )))
    .terminated(tuple((
        opt(parse_whitespace),
        tag("}"),
        opt(parse_whitespace),
    )))
    .parse(input)
}

fn parse_graph<'a>(
    input: &'a str,
    variables: &'_ HashMap<String, String>,
    _deps: &'_ DependencyData,
) -> IResult<&'a str, GraphInstructions> {
    // TODO path:
    //
    // - decode into instructions
    // - make sure paths are expanded as variables
    // - overall:
    //     - map-instructions (DONE, no expand YET)
    //     - group-instructions
    //     - zoom-list (NOT instructions)

    tuple((parse_map_instructions,))
        .preceded_by(tuple((
            opt(parse_whitespace),
            tag("graph"),
            parse_whitespace,
            tag("{"),
            opt(parse_whitespace),
        )))
        .terminated(tuple((
            opt(parse_whitespace),
            tag("}"),
            opt(parse_whitespace),
        )))
        .map(|(map_instructions,)| GraphInstructions { map_instructions }.mapped(variables))
        .parse(input)

    // graph {
    //    map {
    //      ${CHIP_ROOT}/src/app => app::
    //      ${GEN_ROOT} => zapgen::
    //
    //      keep app::
    //      keep zapgen::
    //    }
    //    group {
    //       gn root ${COMPILE_ROOT} target //src/app/* sources ${CHIP_ROOT}
    //       manual test_group {
    //         app::SomeFileName.h
    //         app::OtherName.cpp
    //       }
    //       group_source_header
    //    }
    //    zoom {
    //      test_group
    //      //src/app
    //    }
    // }
}

async fn parse_data(input: &str) -> IResult<&str, ()> {
    let input = match parse_whitespace(input) {
        Ok((data, _)) => data,
        _ => input,
    };

    // First, parse all variables
    let (input, input_vars) = separated_list0(
        parse_whitespace,
        separated_pair(parse_variable_name, tag("="), parse_until_whitespace),
    )
    .parse(input)?;

    let mut variables = HashMap::new();
    for (name, value) in input_vars {
        variables.insert(name.to_string(), expand_variable(value, &variables));
    }

    debug!("Resolved variables: {:#?}", variables);
    debug!("Parsing instructions...");

    let (input, instructions) = tuple((opt(parse_whitespace), parse_input, opt(parse_whitespace)))
        .map(|(_, i, _)| i)
        .parse(input)?;

    debug!("Instructions: {:#?}", instructions);

    let mut dependency_data = DependencyData::default();

    for i in instructions {
        match i {
            InputCommand::IncludesFromCompileDb(cdb) => {
                match parse_compile_database(&expand_variable(&cdb, &variables)).await {
                    Ok(entries) => {
                        for entry in entries {
                            dependency_data.includes.extend(entry.include_directories);
                        }
                    }
                    Err(err) => {
                        error!("Error parsing compile database {}: {:?}", cdb, err);
                    }
                }
            }
            InputCommand::IncludeDirectory(path) => {
                dependency_data
                    .includes
                    .insert(PathBuf::from(expand_variable(&path, &variables)));
            }
            InputCommand::Glob(g) => {
                let glob = match glob::glob(&expand_variable(&g, &variables)) {
                    Ok(value) => value,
                    Err(e) => {
                        error!("Glob error for {}: {:?}", g, e);
                        continue;
                    }
                };
                let includes_array = dependency_data
                    .includes
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>();
                match all_sources_and_includes(glob, &includes_array).await {
                    Ok(data) => dependency_data.files.extend(data),
                    Err(e) => {
                        error!("Include prodcessing for {} failed: {:?}", g, e);
                        continue;
                    }
                }
            }
        }
    }

    // debug!("Dependency data: {:#?}", dependency_data);

    let (input, instructions) = parse_graph(input, &variables, &dependency_data)?;

    debug!("INSTRUCTIONS: {:#?}", instructions);

    Ok((input, ()))
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

    if let Err(e) = parse_data(include_str!("../sample_api.txt")).await {
        error!("PARSE ERROR: {:#?}", e);
    }

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
    debug!("Processing with includes {:#?}", includes);

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
        debug!("GOT: {:?}", r);
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
                debug!("  {:#?}", target);
            }
            info!("Found {} gn targets", items.len());
        }
        Err(e) => {
            error!("GN LOAD ERROR: {:?}", e);
        }
    }
}
