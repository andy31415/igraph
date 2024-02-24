use crate::dependencies::{
    compiledb::parse_compile_database,
    cparse::{all_sources_and_includes, SourceWithIncludes},
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

use tracing::{debug, error};

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
                tag_no_case("includes"),
                parse_whitespace,
                tag_no_case("from"),
                parse_whitespace,
                tag_no_case("compiledb"),
                parse_whitespace,
            )))
            .map(|s| InputCommand::IncludesFromCompileDb(s.into())),
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
    // input {
    //   includes from compiledb ${COMPILE_ROOT}/compile_commands.json
    //   include_dir ${GEN_ROOT}

    //   # Only API will be loaded anyway
    //   glob ${CHIP_ROOT}/src/app/**
    //   glob ${GEN_ROOT}/**
    // }
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupInstruction {
    GroupSourceHeader,
    GroupFromGn {
        gn_root: String,
        target: String,
        source_root: String,
    },
    ManualGroup {
        name: String,
        items: Vec<String>,
    },
}

#[derive(Debug, PartialEq)]
struct GraphInstructions {
    map_instructions: Vec<MapInstruction>,
    group_instructions: Vec<GroupInstruction>,
    zoom_items: Vec<String>,
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
                    } => GroupInstruction::GroupFromGn {
                        gn_root: expand_variable(&gn_root, variables),
                        target,
                        source_root: expand_variable(&source_root, variables),
                    },
                    other => other,
                })
                .collect(),
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

fn parse_manual_group(input: &str) -> IResult<&str, GroupInstruction> {
    tuple((
        parse_until_whitespace
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
    .map(|(name, items)| GroupInstruction::ManualGroup {
        name: name.into(),
        items,
    })
    .terminated(tuple((
        opt(parse_whitespace),
        tag_no_case("}"),
        opt(parse_whitespace),
    )))
    .parse(input)
}

fn parse_group(input: &str) -> IResult<&str, Vec<GroupInstruction>> {
    many0(alt((
        value(
            GroupInstruction::GroupSourceHeader,
            tag_no_case("group_source_header"),
        ),
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
        ))
        .terminated(opt(parse_whitespace))
        .map(
            |(gn_root, target, source_root)| GroupInstruction::GroupFromGn {
                gn_root: gn_root.into(),
                target: target.into(),
                source_root: source_root.into(),
            },
        ),
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

fn parse_zoom(input: &str) -> IResult<&str, Vec<String>> {
    many0(
        is_not("\n\r \t#}")
            .preceded_by(opt(parse_whitespace))
            .map(String::from),
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
    tuple((parse_map_instructions, parse_group, opt(parse_zoom)))
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
        .map(|(map_instructions, group_instructions, zoom)| {
            GraphInstructions {
                map_instructions,
                group_instructions,
                zoom_items: zoom.unwrap_or_default(),
            }
            .mapped(variables)
        })
        .parse(input)
}

pub async fn parse_config_file(input: &str) -> IResult<&str, ()> {
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

    let (input, instructions) = parse_graph(input, &variables)?;

    debug!("INSTRUCTIONS: {:#?}", instructions);

    // TODO operations:
    //   - take dependency_data and prune it based on instructions
    //   - generate a graph with:
    //      - groupings (warn on duplicates)
    //      - dependency links
    //      - zoom-in data (TODO: separate or not?)

    Ok((input, ()))
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
                gn root test/${Foo}/blah target //* sources ${Foo}
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
                            source_root: "srcs1".into()
                        },
                        GroupInstruction::GroupFromGn {
                            gn_root: "test/Bar/blah".into(),
                            target: "//*".into(),
                            source_root: "Bar".into()
                        },
                    ],
                    zoom_items: Vec::default(),
                }
            ))
        );
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
                    "this".to_string(),
                    "is".to_string(),
                    "some".to_string(),
                    "test".to_string()
                ]
            ))
        );

        assert!(parse_zoom("blah").is_err());
    }
}
