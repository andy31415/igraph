use crate::dependencies::{
    compiledb::parse_compile_database,
    cparse::{all_sources_and_includes, extract_includes, SourceWithIncludes},
    gn::load_gn_targets,
    graph::GraphBuilder,
    path_mapper::{PathMapper, PathMapping},
};
use color_eyre::Result;
use color_eyre::{eyre::WrapErr, Report};
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

/// Defines an instruction regarding name mapping
#[derive(Debug, PartialEq, Clone)]
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

#[derive(Debug, PartialEq, Clone)]
pub struct ZoomItem {
    name: String,
    focused: bool,
}

#[derive(Debug, PartialEq, Clone)]
pub enum GroupEdgeEnd {
    From(String),
    To(String),
}

#[derive(Debug, PartialEq, Clone)]
pub enum EdgeColor {
    Regular(String),
    Bold(String),
}

impl EdgeColor {
    pub fn color_name(&self) -> &str {
        match self {
            EdgeColor::Regular(c) => &c,
            EdgeColor::Bold(c) => &c,
        }
    }

    pub fn is_bold(&self) -> bool {
        match self {
            EdgeColor::Regular(_) => false,
            EdgeColor::Bold(_) => true,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct ColorInstruction {
    end: GroupEdgeEnd,
    color: EdgeColor,
}

/// How a config file looks like
#[derive(Debug, PartialEq, Clone)]
enum InputCommand {
    LoadCompileDb {
        path: String,
        load_includes: bool,
        load_sources: bool,
    },
    IncludeDirectory(String),
    Glob(String),
}

#[derive(Debug, PartialEq, Clone)]
struct VariableAssignment {
    name: String,
    value: String,
}

#[derive(Debug, PartialEq, Default, Clone)]
struct GraphInstructions {
    map_instructions: Vec<MapInstruction>,
    group_instructions: Vec<GroupInstruction>,
    color_instructions: Vec<ColorInstruction>,
    zoom_items: Vec<ZoomItem>,
}

/// Defines a full configuration file, with components
/// resolved as much as possible
#[derive(Debug, PartialEq, Clone, Default)]
struct ConfigurationFile {
    /// Fully resolved variables
    variable_map: HashMap<String, String>,

    /// What inputs are to be processed
    input_commands: Vec<InputCommand>,

    /// Instructions to build a braph
    graph: GraphInstructions,
}

fn expand_variable(value: &str, variable_map: &HashMap<String, String>) -> String {
    // expand any occurences of "${name}"
    let mut value = value.to_string();

    loop {
        let replacements = variable_map
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

/// Something that changes by self-expanding variables
///
/// Variable expansion is replacing "${name}" with the content of the
/// key `name` in the variable_map hashmap
trait Expanded {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self;
}

trait ResolveVariables<O> {
    fn resolve_variables(self) -> O;
}

impl ResolveVariables<HashMap<String, String>> for Vec<VariableAssignment> {
    fn resolve_variables(self) -> HashMap<String, String> {
        let mut variable_map = HashMap::new();
        for VariableAssignment { name, value } in self {
            variable_map.insert(name, value.expanded_from(&variable_map));
        }
        variable_map
    }
}

impl Expanded for ZoomItem {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        Self {
            name: self.name.expanded_from(variable_map),
            ..self
        }
    }
}

impl Expanded for GroupEdgeEnd {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        match self {
            GroupEdgeEnd::From(v) => GroupEdgeEnd::From(v.expanded_from(variable_map)),
            GroupEdgeEnd::To(v) => GroupEdgeEnd::To(v.expanded_from(variable_map)),
        }
    }
}

impl Expanded for EdgeColor {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        match self {
            EdgeColor::Regular(c) => EdgeColor::Regular(c.expanded_from(variable_map)),
            EdgeColor::Bold(c) => EdgeColor::Bold(c.expanded_from(variable_map)),
        }
    }
}

impl Expanded for ColorInstruction {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        Self {
            end: self.end.expanded_from(variable_map),
            color: self.color.expanded_from(variable_map),
        }
    }
}

impl Expanded for InputCommand {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        match self {
            InputCommand::LoadCompileDb {
                path,
                load_includes,
                load_sources,
            } => InputCommand::LoadCompileDb {
                path: path.expanded_from(variable_map),
                load_includes,
                load_sources,
            },
            InputCommand::IncludeDirectory(p) => {
                InputCommand::IncludeDirectory(p.expanded_from(variable_map))
            }
            InputCommand::Glob(p) => InputCommand::Glob(p.expanded_from(variable_map)),
        }
    }
}

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

/// Parse an individual comment.
///
/// ```
/// # use include_graph::dependencies::configfile::parse_comment;
///
/// assert_eq!(parse_comment("# foo"), Ok(("", " foo")));
/// assert_eq!(parse_comment("# foo\ntest"), Ok(("\ntest", " foo")));
/// assert_eq!(parse_comment("# foo\n#bar"), Ok(("\n#bar", " foo")));
/// assert!(parse_comment("blah").is_err());
/// assert!(parse_comment("blah # rest").is_err());
///
/// ```
pub fn parse_comment(input: &str) -> IResult<&str, &str> {
    pair(parsed_char('#'), opt(is_not("\n\r")))
        .map(|(_, r)| r.unwrap_or_default())
        .parse(input)
}

/// Parse whitespace until the first non-whitespace character
///
/// Whitespace is considered:
///    - actual space
///    - any comment(s)
///
/// ```
/// # use include_graph::dependencies::configfile::parse_whitespace;
///
/// assert_eq!(parse_whitespace("  \n  foo"), Ok(("foo", ())));
/// assert_eq!(parse_whitespace(" # test"), Ok(("", ())));
/// assert_eq!(parse_whitespace("# rest"), Ok(("", ())));
/// assert_eq!(parse_whitespace("  # test\n  \t# more\n  last\nthing"), Ok(("last\nthing", ())));
///
/// assert!(parse_whitespace("blah").is_err());
///
/// ```
pub fn parse_whitespace(input: &str) -> IResult<&str, ()> {
    value((), many1(alt((multispace1, parse_comment)))).parse(input)
}

fn parse_variable_name(input: &str) -> IResult<&str, &str> {
    is_not("= \t\r\n{}[]()#").parse(input)
}

/// Parse things until the first whitespace character (comments are whitespace).
/// Parsing until end of input is also acceptable (end of input is whitespace)
///
/// ```
/// # use include_graph::dependencies::configfile::parse_until_whitespace;
///
/// assert_eq!(parse_until_whitespace("x  \n  foo"), Ok(("  \n  foo", "x")));
/// assert_eq!(parse_until_whitespace("foo bar"), Ok((" bar", "foo")));
/// assert_eq!(parse_until_whitespace("foo"), Ok(("", "foo")));
/// assert_eq!(parse_until_whitespace("foo# comment"), Ok(("# comment", "foo")));
/// assert_eq!(parse_until_whitespace("foo then a # comment"), Ok((" then a # comment", "foo")));
/// assert!(parse_until_whitespace("\n  foo").is_err());
///
/// ```
pub fn parse_until_whitespace(input: &str) -> IResult<&str, &str> {
    is_not("#\n\r \t").parse(input)
}

fn parse_compiledb(input: &str) -> IResult<&str, InputCommand> {
    #[derive(Clone, Copy, PartialEq)]
    enum Type {
        Includes,
        Sources,
    }
    tuple((
        parse_until_whitespace
            .preceded_by(tuple((
                opt(parse_whitespace),
                tag_no_case("from"),
                parse_whitespace,
                tag_no_case("compiledb"),
                parse_whitespace,
            )))
            .terminated(parse_whitespace),
        separated_list0(
            tuple((
                opt(parse_whitespace),
                tag_no_case(","),
                opt(parse_whitespace),
            )),
            alt((
                value(Type::Includes, tag_no_case("includes")),
                value(Type::Sources, tag_no_case("sources")),
            )),
        )
        .preceded_by(tuple((tag_no_case("load"), opt(parse_whitespace))))
        .terminated(opt(parse_whitespace)),
    ))
    .map(|(path, selections)| InputCommand::LoadCompileDb {
        path: path.into(),
        load_includes: selections.contains(&Type::Includes),
        load_sources: selections.contains(&Type::Sources),
    })
    .parse(input)
}

fn parse_input_command(input: &str) -> IResult<&str, InputCommand> {
    alt((
        parse_compiledb,
        parse_until_whitespace
            .preceded_by(tuple((tag_no_case("glob"), parse_whitespace)))
            .terminated(opt(parse_whitespace))
            .map(|s| InputCommand::Glob(s.into())),
        parse_until_whitespace
            .preceded_by(tuple((tag_no_case("include_dir"), parse_whitespace)))
            .terminated(opt(parse_whitespace))
            .map(|s| InputCommand::IncludeDirectory(s.into())),
    ))
    .parse(input)
}

fn parse_input(input: &str) -> IResult<&str, Vec<InputCommand>> {
    many0(parse_input_command)
        .preceded_by(tuple((
            tag_no_case("input"),
            parse_whitespace,
            tag_no_case("{"),
            opt(parse_whitespace),
        )))
        .terminated(tuple((tag_no_case("}"), opt(parse_whitespace))))
        .parse(input)
}

impl<T> Expanded for Vec<T>
where
    T: Expanded,
{
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        self.into_iter()
            .map(|v| v.expanded_from(variable_map))
            .collect()
    }
}

impl Expanded for String {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        expand_variable(&self, variable_map)
    }
}

impl Expanded for MapInstruction {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        match self {
            MapInstruction::DisplayMap { from, to } => MapInstruction::DisplayMap {
                from: from.expanded_from(variable_map),
                to: to.expanded_from(variable_map),
            },
            MapInstruction::Keep(v) => MapInstruction::Keep(v.expanded_from(variable_map)),
            MapInstruction::Drop(v) => MapInstruction::Drop(v.expanded_from(variable_map)),
        }
    }
}

impl Expanded for GroupInstruction {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        match self {
            GroupInstruction::GroupSourceHeader => self,
            GroupInstruction::GroupFromGn {
                gn_root,
                target,
                source_root,
                ignore_targets,
            } => GroupInstruction::GroupFromGn {
                gn_root: gn_root.expanded_from(variable_map),
                target,
                source_root: source_root.expanded_from(variable_map),
                ignore_targets,
            },
            GroupInstruction::ManualGroup { name, color, items } => {
                GroupInstruction::ManualGroup { name, color, items }
            }
        }
    }
}

impl Expanded for GraphInstructions {
    fn expanded_from(self, variable_map: &HashMap<String, String>) -> Self {
        Self {
            map_instructions: self.map_instructions.expanded_from(variable_map),
            group_instructions: self.group_instructions.expanded_from(variable_map),
            color_instructions: self.color_instructions.expanded_from(variable_map),
            zoom_items: self.zoom_items.expanded_from(variable_map),
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

fn parse_group_by_extension(input: &str) -> IResult<&str, GroupInstruction> {
    // TODO: in the future consider if we should allow a "group these extensions"
    //       instead of automatic
    //
    //       Automatic seems nice because it just strips extensions regardless of
    //       content.
    value(
        GroupInstruction::GroupSourceHeader,
        tag_no_case("group_source_header"),
    )
    .terminated(opt(parse_whitespace))
    .parse(input)
}

fn parse_group(input: &str) -> IResult<&str, Vec<GroupInstruction>> {
    many0(alt((
        parse_group_by_extension,
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

fn parse_color_instruction(input: &str) -> IResult<&str, ColorInstruction> {
    #[derive(Copy, Clone, PartialEq)]
    enum Direction {
        From,
        To,
    }

    tuple((
        alt((
            value(Direction::From, tag_no_case("from")),
            value(Direction::To, tag_no_case("to")),
        ))
        .preceded_by(opt(parse_whitespace))
        .terminated(parse_whitespace),
        parse_until_whitespace.terminated(parse_whitespace),
        opt(tag_no_case("bold").terminated(parse_whitespace)).map(|v| v.is_some()),
        parse_until_whitespace,
    ))
    .map(|(direction, name, bold, color)| ColorInstruction {
        end: match direction {
            Direction::From => GroupEdgeEnd::From(name.into()),
            Direction::To => GroupEdgeEnd::To(name.into()),
        },
        color: if bold {
            EdgeColor::Bold(color.into())
        } else {
            EdgeColor::Regular(color.into())
        },
    })
    .parse(input)
}

fn parse_color_instructions(input: &str) -> IResult<&str, Vec<ColorInstruction>> {
    separated_list0(parse_whitespace, parse_color_instruction)
        .preceded_by(tuple((
            opt(parse_whitespace),
            tag_no_case("color"),
            parse_whitespace,
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

fn parse_graph(input: &str) -> IResult<&str, GraphInstructions> {
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
        |(map_instructions, group_instructions, color_instructions, zoom)| GraphInstructions {
            map_instructions,
            group_instructions,
            color_instructions: color_instructions.unwrap_or_default(),
            zoom_items: zoom.unwrap_or_default(),
        },
    )
    .parse(input)
}

fn parse_variable_assignment(input: &str) -> IResult<&str, VariableAssignment> {
    separated_pair(
        parse_variable_name,
        tag_no_case("=")
            .preceded_by(opt(parse_whitespace))
            .terminated(opt(parse_whitespace)),
        parse_until_whitespace,
    )
    .map(|(name, value)| VariableAssignment {
        name: name.into(),
        value: value.into(),
    })
    .parse(input)
}

fn parse_variable_assignments(input: &str) -> IResult<&str, HashMap<String, String>> {
    separated_list0(parse_whitespace, parse_variable_assignment)
        .preceded_by(opt(parse_whitespace))
        .terminated(opt(parse_whitespace))
        .map(|v| v.resolve_variables())
        .parse(input)
}

fn parse_config(input: &str) -> IResult<&str, ConfigurationFile> {
    tuple((parse_variable_assignments, parse_input, parse_graph))
        .map(|(variable_map, input_commands, graph)| ConfigurationFile {
            input_commands: input_commands.expanded_from(&variable_map),
            graph: graph.expanded_from(&variable_map),
            variable_map,
        })
        .parse(input)
}

pub async fn build_graph(input: &str) -> Result<Graph, Report> {
    let (input, config) = parse_config(input)
        .map_err(|e| Error::ConfigParseError {
            message: format!("Nom error: {:?}", e),
        })
        .wrap_err("Failed to parse with nom")?;

    if !input.is_empty() {
        return Err(Error::ConfigParseError {
            message: format!("Not all input was consumed: {:?}", input),
        }
        .into());
    }

    debug!("Variables: {:#?}", config.variable_map);
    debug!("Input:     {:#?}", config.input_commands);
    debug!("Graph:     {:#?}", config.graph);

    let mut dependency_data = DependencyData::default();

    for i in config.input_commands {
        match i {
            InputCommand::LoadCompileDb {
                path,
                load_includes,
                load_sources,
            } => {
                let entries = match parse_compile_database(&path).await {
                    Ok(entries) => entries,
                    Err(err) => {
                        error!("Error parsing compile database {}: {:?}", path, err);
                        continue;
                    }
                };
                if load_includes {
                    for entry in entries.iter() {
                        dependency_data
                            .includes
                            .extend(entry.include_directories.clone());
                    }
                }
                if load_sources {
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
            }
            InputCommand::IncludeDirectory(path) => {
                dependency_data.includes.insert(PathBuf::from(path));
            }
            InputCommand::Glob(g) => {
                let glob = match glob::glob(&g) {
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

    // set up a path mapper
    let mut mapper = PathMapper::default();
    for i in config.graph.map_instructions.iter() {
        if let MapInstruction::DisplayMap { from, to } = i {
            mapper.add_mapping(PathMapping {
                from: PathBuf::from(from),
                to: to.clone(),
            });
        }
    }
    let keep = config
        .graph
        .map_instructions
        .iter()
        .filter_map(|i| match i {
            MapInstruction::Keep(v) => Some(v),
            _ => None,
        })
        .collect::<HashSet<_>>();

    let drop = config
        .graph
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
    for group_instruction in config.graph.group_instructions {
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
    for item in config.graph.zoom_items {
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

    for i in config.graph.color_instructions {
        match i.end {
            GroupEdgeEnd::From(name) => {
                g.color_from(&name, i.color.color_name(), i.color.is_bold())
            }
            GroupEdgeEnd::To(name) => g.color_to(&name, i.color.color_name(), i.color.is_bold()),
        }
    }

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
        let mut variable_map = HashMap::new();
        variable_map.insert("Foo".into(), "Bar".into());

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
            )
            .map(|(r, g)| { (r, g.expanded_from(&variable_map)) }),
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
    fn test_color_instruction_parsing() {
        assert_eq!(
            parse_color_instruction("from source color"),
            Ok((
                "",
                ColorInstruction {
                    end: GroupEdgeEnd::From("source".into()),
                    color: EdgeColor::Regular("color".into())
                }
            ))
        );

        assert_eq!(
            parse_color_instruction("to destination color"),
            Ok((
                "",
                ColorInstruction {
                    end: GroupEdgeEnd::To("destination".into()),
                    color: EdgeColor::Regular("color".into())
                }
            ))
        );

        assert_eq!(
            parse_color_instruction("From x y"),
            Ok((
                "",
                ColorInstruction {
                    end: GroupEdgeEnd::From("x".into()),
                    color: EdgeColor::Regular("y".into())
                }
            ))
        );

        assert_eq!(
            parse_color_instruction("#comment\n  TO a bold b"),
            Ok((
                "",
                ColorInstruction {
                    end: GroupEdgeEnd::To("a".into()),
                    color: EdgeColor::Bold("b".into())
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
            from a bold b
         }"
            ),
            Ok((
                "",
                vec![
                    ColorInstruction {
                        end: GroupEdgeEnd::From("x".into()),
                        color: EdgeColor::Regular("y".into()),
                    },
                    ColorInstruction {
                        end: GroupEdgeEnd::To("q".into()),
                        color: EdgeColor::Regular("r".into()),
                    },
                    ColorInstruction {
                        end: GroupEdgeEnd::From("a".into()),
                        color: EdgeColor::Bold("b".into()),
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
    fn test_parse_glob() {
        assert_eq!(
            parse_input_command("glob a/b/**/*"),
            Ok(("", InputCommand::Glob("a/b/**/*".into())))
        );

        assert_eq!(
            parse_input_command("glob a/x/**/*.h # should consume whitespace\n\n  \n\t\n  "),
            Ok(("", InputCommand::Glob("a/x/**/*.h".into())))
        );
    }

    #[test]
    fn test_parse_include_dir() {
        assert_eq!(
            parse_input_command("include_dir a/b/**/*"),
            Ok(("", InputCommand::IncludeDirectory("a/b/**/*".into())))
        );

        assert_eq!(
            parse_input_command("include_dir a/x/**/*.h # should consume whitespace\n\n  \n\t\n  "),
            Ok(("", InputCommand::IncludeDirectory("a/x/**/*.h".into())))
        );
    }

    #[test]
    fn test_parse_compiledb() {
        assert_eq!(
            parse_compiledb("from compiledb foo load includes"),
            Ok((
                "",
                InputCommand::LoadCompileDb {
                    path: "foo".into(),
                    load_includes: true,
                    load_sources: false
                }
            ))
        );

        assert_eq!(
            parse_compiledb("from compiledb bar load sources"),
            Ok((
                "",
                InputCommand::LoadCompileDb {
                    path: "bar".into(),
                    load_includes: false,
                    load_sources: true
                }
            ))
        );

        assert_eq!(
            parse_compiledb("from compiledb bar load sources, includes, sources"),
            Ok((
                "",
                InputCommand::LoadCompileDb {
                    path: "bar".into(),
                    load_includes: true,
                    load_sources: true
                }
            ))
        );

        assert_eq!(
            parse_compiledb(
                "
                # This is a comment (and whitespace) prefix
                from compiledb x/y/z load sources, sources\n# space/comment suffix\n"
            ),
            Ok((
                "",
                InputCommand::LoadCompileDb {
                    path: "x/y/z".into(),
                    load_includes: false,
                    load_sources: true
                }
            ))
        );

        assert_eq!(
            parse_compiledb(
                "
                from compiledb x/y/z load sources, sources\nremaining"
            ),
            Ok((
                "remaining",
                InputCommand::LoadCompileDb {
                    path: "x/y/z".into(),
                    load_includes: false,
                    load_sources: true
                }
            ))
        );
    }

    #[test]
    fn test_parse_input() {
        assert_eq!(
            parse_input(
                "input {
           from compiledb some_compile_db.json load includes
           include_dir foo
           from compiledb some_compile_db.json load sources

           glob xyz/**/*

           include_dir bar
           from compiledb another.json load sources, includes

           glob final/**/*
           glob blah/**/*
        }"
            ),
            Ok((
                "",
                vec![
                    InputCommand::LoadCompileDb {
                        path: "some_compile_db.json".into(),
                        load_includes: true,
                        load_sources: false
                    },
                    InputCommand::IncludeDirectory("foo".into()),
                    InputCommand::LoadCompileDb {
                        path: "some_compile_db.json".into(),
                        load_includes: false,
                        load_sources: true
                    },
                    InputCommand::Glob("xyz/**/*".into()),
                    InputCommand::IncludeDirectory("bar".into()),
                    InputCommand::LoadCompileDb {
                        path: "another.json".into(),
                        load_includes: true,
                        load_sources: true
                    },
                    InputCommand::Glob("final/**/*".into()),
                    InputCommand::Glob("blah/**/*".into()),
                ]
            ))
        );
    }

    #[test]
    fn test_variable_assignments() {
        assert_eq!(
            parse_variable_assignments(
                "
             a = b
             x=y
             z=${a}${x}
             ab=test
             other=${a${a}}ing
           "
            ),
            {
                let mut expected = HashMap::new();
                expected.insert("a".into(), "b".into());
                expected.insert("x".into(), "y".into());
                expected.insert("z".into(), "by".into());
                expected.insert("ab".into(), "test".into());
                expected.insert("other".into(), "testing".into());
                Ok(("", expected))
            }
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
