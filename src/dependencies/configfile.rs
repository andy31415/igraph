use crate::dependencies::{
    compiledb::parse_compile_database,
    cparse::{all_sources_and_includes, extract_includes, SourceWithIncludes},
    gn::load_gn_targets,
    graph::GraphBuilder,
    path_mapper::{PathMapper, PathMapping},
};
use nom::{
    branch::alt,
    bytes::complete::{is_not, tag_no_case},
    character::complete::{char as parsed_char, multispace1},
    combinator::{opt, value},
    multi::{many0, many1, separated_list0},
    sequence::{pair, separated_pair, tuple},
    IResult, Parser,
};
use nom_supreme::ParserExt;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use tracing::{debug, error, info};

use super::{error::Error, graph::Graph};

#[derive(Debug, Default)]
struct DependencyData {
    includes: HashSet<PathBuf>,
    files: Vec<SourceWithIncludes>,
}

/// Pretty-print dependency data.
///
/// Wrapped as a separate struct to support lazy formatting
struct FullFileList<'a> {
    dependencies: &'a DependencyData,
}

impl<'a> FullFileList<'a> {
    pub fn new(dependencies: &'a DependencyData) -> Self {
        Self { dependencies }
    }
}

impl<'a> std::fmt::Display for FullFileList<'a> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.write_str("Processed files:\n")?;
        for f in self.dependencies.files.iter() {
            fmt.write_fmt(format_args!("  {:?}\n", f.path))?;
        }
        Ok(())
    }
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
    pair(parsed_char('#'), opt(is_not("\n\r")))
        .map(|(_, r)| r.unwrap_or_default())
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

#[derive(Debug, PartialEq, Clone)]
enum InputCommand {
    IncludesFromCompileDb(String),
    SourcesFromCompileDb(String),
    IncludeDirectory(String),
    Glob(String),
}

fn parse_input_command(input: &str) -> IResult<&str, InputCommand> {
    alt((
        parse_until_whitespace
            .preceded_by(tuple((
                tag_no_case("includes"),
                parse_whitespace,
                tag_no_case("from"),
                parse_whitespace,
                tag_no_case("compiledb"),
                parse_whitespace,
            )))
            .map(|s| InputCommand::IncludesFromCompileDb(s.into())),
        parse_until_whitespace
            .preceded_by(tuple((
                tag_no_case("sources"),
                parse_whitespace,
                tag_no_case("from"),
                parse_whitespace,
                tag_no_case("compiledb"),
                parse_whitespace,
            )))
            .map(|s| InputCommand::SourcesFromCompileDb(s.into())),
        parse_until_whitespace
            .preceded_by(tuple((tag_no_case("glob"), parse_whitespace)))
            .map(|s| InputCommand::Glob(s.into())),
        parse_until_whitespace
            .preceded_by(tuple((tag_no_case("include_dir"), parse_whitespace)))
            .map(|s| InputCommand::IncludeDirectory(s.into())),
    ))
    .parse(input)
}

fn parse_input(input: &str) -> IResult<&str, Vec<InputCommand>> {
    tuple((
        tuple((
            tag_no_case("input"),
            parse_whitespace,
            tag_no_case("{"),
            opt(parse_whitespace),
        )),
        separated_list0(parse_whitespace, parse_input_command),
        tuple((
            opt(parse_whitespace),
            tag_no_case("}"),
            opt(parse_whitespace),
        )),
    ))
    .map(|(_, l, _)| l)
    .parse(input)
}

/// Defines an instruction regarding name mapping
#[derive(Debug, PartialEq)]
pub enum MapInstruction {
    DisplayMap { from: String, to: String },
    Keep(String),
    Drop(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupInstruction {
    GroupSourceHeader,
    GroupFromGn {
        gn_root: String,
        target: String,
        source_root: String,
        ignore_targets: HashSet<String>,
    },
    ManualGroup {
        name: String,
        color: Option<String>,
        items: Vec<String>,
    },
}

#[derive(Debug, PartialEq)]
pub struct ZoomItem {
    name: String,
    focused: bool,
}

#[derive(Debug, PartialEq)]
pub enum GroupEdgeEnd {
    From(String),
    To(String),
}

#[derive(Debug, PartialEq)]
pub struct ColorInstruction {
    end: GroupEdgeEnd,
    color: String,
}

#[derive(Debug, PartialEq, Default)]
struct GraphInstructions {
    map_instructions: Vec<MapInstruction>,
    group_instructions: Vec<GroupInstruction>,
    color_instructions: Vec<ColorInstruction>,
    zoom_items: Vec<ZoomItem>,
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
            group_instructions: self
                .group_instructions
                .into_iter()
                .map(|instruction| match instruction {
                    GroupInstruction::GroupFromGn {
                        gn_root,
                        target,
                        source_root,
                        ignore_targets,
                    } => GroupInstruction::GroupFromGn {
                        gn_root: expand_variable(&gn_root, variables),
                        target,
                        source_root: expand_variable(&source_root, variables),
                        ignore_targets,
                    },
                    other => other,
                })
                .collect(),
            color_instructions: self.color_instructions,
            zoom_items: self.zoom_items,
        }
    }
}

fn parse_map_instructions(input: &str) -> IResult<&str, Vec<MapInstruction>> {
    many0(
        alt((
            separated_pair(
                parse_until_whitespace,
                tuple((parse_whitespace, tag_no_case("=>"), parse_whitespace)),
                parse_until_whitespace,
            )
            .map(|(from, to)| MapInstruction::DisplayMap {
                from: from.into(),
                to: to.into(),
            }),
            parse_until_whitespace
                .preceded_by(tuple((
                    opt(parse_whitespace),
                    tag_no_case("keep"),
                    parse_whitespace,
                )))
                .map(|s| MapInstruction::Keep(s.into())),
            parse_until_whitespace
                .preceded_by(tuple((
                    opt(parse_whitespace),
                    tag_no_case("drop"),
                    parse_whitespace,
                )))
                .map(|s| MapInstruction::Drop(s.into())),
        ))
        .terminated(parse_whitespace),
    )
    .preceded_by(tuple((
        tag_no_case("map"),
        parse_whitespace,
        tag_no_case("{"),
        parse_whitespace,
    )))
    .terminated(tuple((
        opt(parse_whitespace),
        tag_no_case("}"),
        opt(parse_whitespace),
    )))
    .parse(input)
}

fn parse_gn_target(input: &str) -> IResult<&str, GroupInstruction> {
    tuple((
        parse_until_whitespace.preceded_by(tuple((
            tag_no_case("gn"),
            parse_whitespace,
            tag_no_case("root"),
            parse_whitespace,
        ))),
        parse_until_whitespace.preceded_by(tuple((
            parse_whitespace,
            tag_no_case("target"),
            parse_whitespace,
        ))),
        parse_until_whitespace.preceded_by(tuple((
            parse_whitespace,
            tag_no_case("sources"),
            parse_whitespace,
        ))),
        opt(parse_target_list
            .preceded_by(tuple((
                parse_whitespace,
                tag_no_case("ignore"),
                parse_whitespace,
                tag_no_case("targets"),
                opt(parse_whitespace),
                tag_no_case("{"),
            )))
            .terminated(tuple((
                opt(parse_whitespace),
                tag_no_case("}"),
                opt(parse_whitespace),
            )))),
    ))
    .terminated(opt(parse_whitespace))
    .map(
        |(gn_root, target, source_root, ignore_targets)| GroupInstruction::GroupFromGn {
            gn_root: gn_root.into(),
            target: target.into(),
            source_root: source_root.into(),
            ignore_targets: ignore_targets
                .map(|v| v.into_iter().map(|s| s.into()).collect())
                .unwrap_or_default(),
        },
    )
    .parse(input)
}

fn parse_manual_group(input: &str) -> IResult<&str, GroupInstruction> {
    tuple((
        tuple((
            parse_until_whitespace,
            opt(parse_until_whitespace.preceded_by(tuple((
                parse_whitespace,
                tag_no_case("color"),
                opt(parse_whitespace),
            )))),
        ))
        .preceded_by(tuple((
            opt(parse_whitespace),
            tag_no_case("manual"),
            opt(parse_whitespace),
        )))
        .terminated(tuple((opt(parse_whitespace), tag_no_case("{")))),
        many0(
            is_not("\n\r \t#}")
                .preceded_by(opt(parse_whitespace))
                .map(String::from),
        ),
    ))
    .map(|((name, color), items)| GroupInstruction::ManualGroup {
        name: name.into(),
        color: color.map(|c| c.into()),
        items,
    })
    .terminated(tuple((
        opt(parse_whitespace),
        tag_no_case("}"),
        opt(parse_whitespace),
    )))
    .parse(input)
}

fn parse_target_list(input: &str) -> IResult<&str, Vec<&str>> {
    many0(is_not("\n\r \t#}").preceded_by(opt(parse_whitespace)))
        .terminated(opt(parse_whitespace))
        .parse(input)
}

fn parse_group(input: &str) -> IResult<&str, Vec<GroupInstruction>> {
    many0(alt((
        value(
            GroupInstruction::GroupSourceHeader,
            tag_no_case("group_source_header"),
        ),
        parse_gn_target,
        parse_manual_group,
    )))
    .preceded_by(tuple((
        opt(parse_whitespace),
        tag_no_case("group"),
        opt(parse_whitespace),
        tag_no_case("{"),
        opt(parse_whitespace),
    )))
    .terminated(tuple((
        opt(parse_whitespace),
        tag_no_case("}"),
        opt(parse_whitespace),
    )))
    .parse(input)
}

fn parse_color_instructions(input: &str) -> IResult<&str, Vec<ColorInstruction>> {
    many0(alt((
        tuple((
            parse_until_whitespace,
            parse_until_whitespace.preceded_by(parse_whitespace),
        ))
        .preceded_by(tuple((
            opt(parse_whitespace),
            tag_no_case("from"),
            parse_whitespace,
        )))
        .map(|(name, color)| ColorInstruction {
            color: color.into(),
            end: GroupEdgeEnd::From(name.into()),
        }),
        tuple((
            parse_until_whitespace,
            parse_until_whitespace.preceded_by(parse_whitespace),
        ))
        .preceded_by(tuple((
            opt(parse_whitespace),
            tag_no_case("to"),
            parse_whitespace,
        )))
        .map(|(name, color)| ColorInstruction {
            color: color.into(),
            end: GroupEdgeEnd::To(name.into()),
        }),
    )))
    .preceded_by(tuple((
        opt(parse_whitespace),
        tag_no_case("color"),
        opt(parse_whitespace),
        tag_no_case("edges"),
        opt(parse_whitespace),
        tag_no_case("{"),
        opt(parse_whitespace),
    )))
    .terminated(tuple((
        opt(parse_whitespace),
        tag_no_case("}"),
        opt(parse_whitespace),
    )))
    .parse(input)
}

fn parse_zoom(input: &str) -> IResult<&str, Vec<ZoomItem>> {
    many0(
        tuple((
            opt(tag_no_case("focus:").terminated(parse_whitespace)),
            is_not("\n\r \t#}").terminated(opt(parse_whitespace)),
        ))
        .map(|(focus, name)| ZoomItem {
            name: name.into(),
            focused: focus.is_some(),
        }),
    )
    .preceded_by(tuple((
        opt(parse_whitespace),
        tag_no_case("zoom"),
        opt(parse_whitespace),
        tag_no_case("{"),
        opt(parse_whitespace),
    )))
    .terminated(tuple((
        opt(parse_whitespace),
        tag_no_case("}"),
        opt(parse_whitespace),
    )))
    .parse(input)
}

fn parse_graph<'a>(
    input: &'a str,
    variables: &'_ HashMap<String, String>,
) -> IResult<&'a str, GraphInstructions> {
    tuple((
        parse_map_instructions,
        parse_group,
        opt(parse_color_instructions),
        opt(parse_zoom),
    ))
    .preceded_by(tuple((
        opt(parse_whitespace),
        tag_no_case("graph"),
        parse_whitespace,
        tag_no_case("{"),
        opt(parse_whitespace),
    )))
    .terminated(tuple((
        opt(parse_whitespace),
        tag_no_case("}"),
        opt(parse_whitespace),
    )))
    .map(
        |(map_instructions, group_instructions, color_instructions, zoom)| {
            GraphInstructions {
                map_instructions,
                group_instructions,
                color_instructions: color_instructions.unwrap_or_default(),
                zoom_items: zoom.unwrap_or_default(),
            }
            .mapped(variables)
        },
    )
    .parse(input)
}

pub async fn parse_config_file(input: &str) -> Result<Graph, Error> {
    let input = match parse_whitespace(input) {
        Ok((data, _)) => data,
        _ => input,
    };

    // First, parse all variables
    let (input, input_vars) = separated_list0(
        parse_whitespace,
        separated_pair(
            parse_variable_name,
            tag_no_case("="),
            parse_until_whitespace,
        ),
    )
    .parse(input)
    .map_err(|e| Error::ConfigParseError {
        message: format!("Nom error: {:?}", e),
    })?;

    let mut variables = HashMap::new();
    for (name, value) in input_vars {
        variables.insert(name.to_string(), expand_variable(value, &variables));
    }

    debug!("Resolved variables: {:#?}", variables);
    debug!("Parsing instructions...");

    let (input, instructions) = tuple((opt(parse_whitespace), parse_input, opt(parse_whitespace)))
        .map(|(_, i, _)| i)
        .parse(input)
        .map_err(|e| Error::ConfigParseError {
            message: format!("Nom error: {:?}", e),
        })?;

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
            InputCommand::SourcesFromCompileDb(cdb) => {
                match parse_compile_database(&expand_variable(&cdb, &variables)).await {
                    Ok(entries) => {
                        let includes_array = dependency_data
                            .includes
                            .clone()
                            .into_iter()
                            .collect::<Vec<_>>();
                        for entry in entries {
                            match extract_includes(&entry.file_path, &includes_array).await {
                                Ok(includes) => {
                                    dependency_data.files.push(SourceWithIncludes {
                                        path: entry.file_path,
                                        includes,
                                    });
                                }
                                Err(e) => {
                                    error!(
                                        "Includee extraction for {:?} failed: {:?}",
                                        &entry.file_path, e
                                    );
                                }
                            };
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
                    Ok(data) => {
                        if data.is_empty() {
                            error!("GLOB {:?} resulted in EMPTY file list!", g);
                        }
                        dependency_data.files.extend(data)
                    }
                    Err(e) => {
                        error!("Include prodcessing for {} failed: {:?}", g, e);
                        continue;
                    }
                }
            }
        }
    }

    let (input, instructions) =
        parse_graph(input, &variables).map_err(|e| Error::ConfigParseError {
            message: format!("Nom error: {:?}", e),
        })?;

    if !input.is_empty() {
        return Err(Error::ConfigParseError {
            message: format!("Not all input was consumed: {:?}", input),
        });
    }

    debug!("INSTRUCTIONS: {:#?}", instructions);

    // set up a path mapper
    let mut mapper = PathMapper::default();
    for i in instructions.map_instructions.iter() {
        if let MapInstruction::DisplayMap { from, to } = i {
            mapper.add_mapping(PathMapping {
                from: PathBuf::from(from),
                to: to.clone(),
            });
        }
    }
    let keep = instructions
        .map_instructions
        .iter()
        .filter_map(|i| match i {
            MapInstruction::Keep(v) => Some(v),
            _ => None,
        })
        .collect::<HashSet<_>>();

    let drop = instructions
        .map_instructions
        .iter()
        .filter_map(|i| match i {
            MapInstruction::Drop(v) => Some(v),
            _ => None,
        })
        .collect::<HashSet<_>>();

    info!(target: "full-file-list", "Procesed files: {}", FullFileList::new(&dependency_data));

    // Dependency data is prunned based on instructions
    let mut g = GraphBuilder::new(
        dependency_data
            .files
            .iter()
            .flat_map(|f| f.includes.iter().chain(std::iter::once(&f.path)))
            .filter_map(|path| {
                mapper.try_map(path).map(|to| PathMapping {
                    from: path.clone(),
                    to,
                })
            })
            .filter(|m| keep.iter().any(|prefix| m.to.starts_with(*prefix)))
            .filter(|m| drop.iter().all(|prefix| !m.to.starts_with(*prefix))),
    );

    // define all the groups
    for group_instruction in instructions.group_instructions {
        match group_instruction {
            GroupInstruction::GroupSourceHeader => {
                g.group_extensions(&["h", "cpp", "hpp", "c", "cxx"]);
            }
            GroupInstruction::GroupFromGn {
                gn_root,
                target,
                source_root,
                ignore_targets,
            } => match load_gn_targets(
                &PathBuf::from(gn_root),
                &PathBuf::from(source_root),
                &target,
            )
            .await
            {
                Ok(targets) => g.add_groups_from_gn(targets, ignore_targets),
                Err(e) => error!("Failed to load GN targets: {:?}", e),
            },
            GroupInstruction::ManualGroup { name, color, items } => {
                // items here are mapped, so we have to invert the map to get
                // the actual name...
                g.define_group(
                    &name,
                    color.as_deref().unwrap_or("orange"),
                    items.into_iter().filter_map(|n| mapper.try_invert(&n)),
                );
            }
        }
    }

    // mark what is zoomed in ...
    for item in instructions.zoom_items {
        g.zoom_in(&item.name, item.focused)
    }

    for dep in dependency_data.files {
        if !g.known_path(&dep.path) {
            continue;
        }
        for dest in dep.includes {
            if !g.known_path(&dest) {
                continue;
            }
            g.add_link(&dep.path, &dest);
        }
    }
    
    for i in instructions.color_instructions {
        match i.end {
            GroupEdgeEnd::From(name) => g.color_from(&name, &i.color),
            GroupEdgeEnd::To(name) => g.color_to(&name, &i.color),
        }
    };

    debug!("Final builder: {:#?}", g);

    Ok(g.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comment_parsing() {
        assert_eq!(parse_comment("#abc\r\nhello"), Ok(("\r\nhello", "abc")));
        assert!(parse_comment("not a comment").is_err());
        assert!(parse_comment("comment later # like here").is_err());
    }

    #[test]
    fn test_gn_target() {
        assert_eq!(
            parse_gn_target("gn root test1 target //my/target/* sources srcs1"),
            Ok((
                "",
                GroupInstruction::GroupFromGn {
                    gn_root: "test1".into(),
                    target: "//my/target/*".into(),
                    source_root: "srcs1".into(),
                    ignore_targets: HashSet::new(),
                },
            ))
        );

        assert_eq!(
            parse_gn_target("gn root test1 target //my/target/* sources srcs1 ignore targets {}"),
            Ok((
                "",
                GroupInstruction::GroupFromGn {
                    gn_root: "test1".into(),
                    target: "//my/target/*".into(),
                    source_root: "srcs1".into(),
                    ignore_targets: HashSet::new(),
                },
            ))
        );

        assert_eq!(
            parse_gn_target(
                "gn root test1 target //my/target/* sources srcs1 ignore targets{
            }"
            ),
            Ok((
                "",
                GroupInstruction::GroupFromGn {
                    gn_root: "test1".into(),
                    target: "//my/target/*".into(),
                    source_root: "srcs1".into(),
                    ignore_targets: HashSet::new(),
                },
            ))
        );

        assert_eq!(
            parse_gn_target(
                "gn root test1 target //my/target/* sources srcs1 ignore targets{
                a b
                c
                d
            }"
            ),
            Ok((
                "",
                GroupInstruction::GroupFromGn {
                    gn_root: "test1".into(),
                    target: "//my/target/*".into(),
                    source_root: "srcs1".into(),
                    ignore_targets: vec!["a", "b", "c", "d"]
                        .into_iter()
                        .map(String::from)
                        .collect(),
                },
            ))
        );
    }

    #[test]
    fn test_manual_group() {
        assert_eq!(
            parse_manual_group(
                "
            manual some/name::special {
                file1
                file2
                another/file::test
            }
            "
            ),
            Ok((
                "",
                GroupInstruction::ManualGroup {
                    name: "some/name::special".into(),
                    color: None,
                    items: vec!["file1".into(), "file2".into(), "another/file::test".into(),]
                }
            ))
        );

        assert_eq!(
            parse_manual_group(
                "
            manual some/name::special color red {
                file1
                file2
                another/file::test
            }
            "
            ),
            Ok((
                "",
                GroupInstruction::ManualGroup {
                    name: "some/name::special".into(),
                    color: Some("red".into()),
                    items: vec!["file1".into(), "file2".into(), "another/file::test".into(),]
                }
            ))
        );
    }

    #[test]
    fn test_gn_instruction() {
        let mut variables = HashMap::new();
        variables.insert("Foo".into(), "Bar".into());

        assert_eq!(
            parse_graph(
                "
        graph {
              map {
              }
   
              group {
                gn root test1 target //my/target/* sources srcs1
                gn root test/${Foo}/blah target //* sources ${Foo} ignore targets {
                    //ignore1
                    //ignore:other
                }
              }
        }
        ",
                &variables
            ),
            Ok((
                "",
                GraphInstructions {
                    map_instructions: Vec::default(),
                    group_instructions: vec![
                        GroupInstruction::GroupFromGn {
                            gn_root: "test1".into(),
                            target: "//my/target/*".into(),
                            source_root: "srcs1".into(),
                            ignore_targets: HashSet::new(),
                        },
                        GroupInstruction::GroupFromGn {
                            gn_root: "test/Bar/blah".into(),
                            target: "//*".into(),
                            source_root: "Bar".into(),
                            ignore_targets: {
                                let mut h = HashSet::new();
                                h.insert("//ignore1".into());
                                h.insert("//ignore:other".into());
                                h
                            }
                        },
                    ],
                    ..Default::default()
                }
            ))
        );
    }

    #[test]
    fn test_color_instructions_parsing() {
        assert_eq!(
            parse_color_instructions("color edges {}"),
            Ok(("", Vec::default()))
        );
        assert_eq!(
            parse_color_instructions(" #comment\ncolor edges {  \n  }\n#more comments\n   \n"),
            Ok(("", Vec::default()))
        );

        assert_eq!(
            parse_color_instructions(
                "
         #comment
         color edges {
            from x y
            to q r
            from a b
         }"
            ),
            Ok((
                "",
                vec![
                    ColorInstruction {
                        end: GroupEdgeEnd::From("x".into()),
                        color: "y".into(),
                    },
                    ColorInstruction {
                        end: GroupEdgeEnd::To("q".into()),
                        color: "r".into(),
                    },
                    ColorInstruction {
                        end: GroupEdgeEnd::From("a".into()),
                        color: "b".into(),
                    },
                ]
            ))
        );

        assert_eq!(
            parse_zoom(
                "
         #comment
         zoom{
            normal
            focus: thisone
            not this
         }"
            ),
            Ok((
                "",
                vec![
                    ZoomItem {
                        name: "normal".to_string(),
                        focused: false
                    },
                    ZoomItem {
                        name: "thisone".to_string(),
                        focused: true
                    },
                    ZoomItem {
                        name: "not".to_string(),
                        focused: false
                    },
                    ZoomItem {
                        name: "this".to_string(),
                        focused: false
                    },
                ]
            ))
        );

        assert!(parse_zoom("blah").is_err());
    }

    #[test]
    fn test_zoom_parsing() {
        assert_eq!(parse_zoom("zoom{}"), Ok(("", Vec::default())));
        assert_eq!(
            parse_zoom(" #comment\nzoom {  \n  }\n#more comments\n   \n"),
            Ok(("", Vec::default()))
        );

        assert_eq!(
            parse_zoom(
                "
         #comment
         zoom{
            this
            is some #notice that whitespace matters and NOT newlines
            test
         }"
            ),
            Ok((
                "",
                vec![
                    ZoomItem {
                        name: "this".to_string(),
                        focused: false
                    },
                    ZoomItem {
                        name: "is".to_string(),
                        focused: false
                    },
                    ZoomItem {
                        name: "some".to_string(),
                        focused: false
                    },
                    ZoomItem {
                        name: "test".to_string(),
                        focused: false
                    },
                ]
            ))
        );

        assert_eq!(
            parse_zoom(
                "
         #comment
         zoom{
            normal
            focus: thisone
            not this
         }"
            ),
            Ok((
                "",
                vec![
                    ZoomItem {
                        name: "normal".to_string(),
                        focused: false
                    },
                    ZoomItem {
                        name: "thisone".to_string(),
                        focused: true
                    },
                    ZoomItem {
                        name: "not".to_string(),
                        focused: false
                    },
                    ZoomItem {
                        name: "this".to_string(),
                        focused: false
                    },
                ]
            ))
        );

        assert!(parse_zoom("blah").is_err());
    }

    #[test]
    fn test_parse_target_list() {
        assert_eq!(parse_target_list(""), Ok(("", vec![])));
        assert_eq!(parse_target_list("    "), Ok(("", vec![])));
        assert_eq!(parse_target_list("a b c"), Ok(("", vec!["a", "b", "c"])));
        assert_eq!(
            parse_target_list("  a  \n\n   b\n   c\n\n"),
            Ok(("", vec!["a", "b", "c"]))
        );
        // should not consume the ending brace
        assert_eq!(parse_target_list("}"), Ok(("}", vec![])));
        assert_eq!(parse_target_list("a b c }"), Ok(("}", vec!["a", "b", "c"])));
    }

    #[test]
    fn test_parse_input() {
        assert_eq!(
            parse_input(
                "input {
           includes from compiledb some_compile_db.json
           include_dir foo
           sources from compiledb some_compile_db.json
           
           glob xyz/**/*

           include_dir bar
           includes from compiledb another.json
            
           glob final/**/*
           glob blah/**/*
        }"
            ),
            Ok((
                "",
                vec![
                    InputCommand::IncludesFromCompileDb("some_compile_db.json".into()),
                    InputCommand::IncludeDirectory("foo".into()),
                    InputCommand::SourcesFromCompileDb("some_compile_db.json".into()),
                    InputCommand::Glob("xyz/**/*".into()),
                    InputCommand::IncludeDirectory("bar".into()),
                    InputCommand::IncludesFromCompileDb("another.json".into()),
                    InputCommand::Glob("final/**/*".into()),
                    InputCommand::Glob("blah/**/*".into()),
                ]
            ))
        );
    }

    #[test]
    fn test_expand_var() {
        let mut vars = HashMap::new();
        vars.insert("foo".into(), "bar".into());
        vars.insert("another".into(), "one".into());
        vars.insert("test".into(), "1234".into());
        vars.insert("theone".into(), "final".into());

        assert_eq!(expand_variable("xyz", &vars), "xyz");
        assert_eq!(expand_variable("${foo}", &vars), "bar");
        assert_eq!(expand_variable("${another}", &vars), "one");
        assert_eq!(
            expand_variable("${foo}/${another}/${foo}", &vars),
            "bar/one/bar"
        );
        assert_eq!(expand_variable("${the${another}}", &vars), "final");
    }
}
