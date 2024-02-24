use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::Serialize;
use tera::{Context, Tera, Value};
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tracing::{debug, error, warn};

use super::{error::Error, gn::GnTarget, path_mapper::PathMapping};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct MappedNode {
    // unique id
    pub id: String,

    // actual file this references
    pub path: PathBuf,

    // mapped name for display
    pub display_name: String,
}

/// A group of related items.
///
/// MAY also be a singular item inside, however a graph is generally
/// a group of named items
#[derive(Debug, PartialEq, Serialize)]
pub struct Group {
    /// nice display name
    pub name: String,

    /// are the nodes expanded out
    pub zoomed: bool,

    /// what are the nodes
    pub nodes: HashSet<MappedNode>,
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Serialize)]
pub struct LinkNode {
    pub group_id: String,
    pub node_id: Option<String>,
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Serialize)]
pub struct GraphLink {
    pub from: LinkNode,
    pub to: LinkNode,
}

#[derive(Debug, Default, Serialize)]
pub struct Graph {
    groups: HashMap<String, Group>,
    links: HashSet<GraphLink>,
}

impl Graph {
    pub async fn write_dot<D>(&self, dest: D) -> Result<(), Error>
    where
        D: AsyncWrite + Unpin,
    {
        let mut writer = BufWriter::new(dest);

        let mut tera = Tera::default();
        tera.add_raw_template("dot_template", include_str!("dot.template"))
            .map_err(Error::RenderError)?;

        tera.register_filter(
            "link_target",
            |n: &Value, _: &HashMap<String, Value>| -> tera::Result<Value> {
                let m = match n {
                    Value::Object(o) => o,
                    _ => return Ok(Value::Null),
                };
                let g = n.get("group_id").expect("Must have group id");
                let n = n.get("node_id").expect("Must have group id");

                match (g, n) {
                    (Value::String(group), Value::String(node)) => {
                        Ok(Value::String(format!("{}.{}", group, node)))
                    }
                    (Value::String(group), Value::Null) => Ok(Value::String(group.clone())),
                    _ => Ok(Value::Null),
                }
            },
        );

        writer
            .write(
                tera.render(
                    "dot_template",
                    &Context::from_serialize(&self).map_err(Error::RenderError)?,
                )
                .map_err(Error::RenderError)?
                .to_string()
                .as_bytes(),
            )
            .await
            .map_err(|source| Error::AsyncIOError {
                source,
                message: "Error writing.",
            })?;
        writer.flush().await.map_err(|source| Error::AsyncIOError {
            source,
            message: "Error flushing writer.",
        })
    }
}

#[derive(Debug, Default)]
pub struct GraphBuilder {
    /// Actual graph being built
    graph: Graph,

    /// known path maps, keyed for fast access to the mapped name
    path_maps: HashMap<PathBuf, PathMapping>,

    /// map a group name
    group_name_to_id: HashMap<String, String>,

    /// where nodes are placed
    placement_maps: HashMap<PathBuf, LinkNode>,

    // what things were zoomed in
    zoomed_ids: HashSet<String>,
}

impl GraphBuilder {
    pub fn new(paths: impl Iterator<Item = PathMapping>) -> Self {
        Self {
            path_maps: paths.map(|v| (v.from.clone(), v)).collect(),
            ..Default::default()
        }
    }

    pub fn known_path(&self, path: &Path) -> bool {
        self.path_maps.contains_key(path)
    }

    pub fn group_extensions(&mut self, extensions: &[&str]) {
        // Get every single possible grouping
        let groups = self
            .path_maps
            .keys()
            .map(|p| p.with_extension(""))
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|stem| {
                extensions
                    .iter()
                    .map(|e| stem.with_extension(e))
                    .filter(|p| self.known_path(p))
                    .filter(|p| !self.placement_maps.contains_key(p))
                    .collect::<Vec<_>>()
            })
            .filter(|e| e.len() > 1)
            .collect::<Vec<_>>();

        for group in groups {
            let mut name = self
                .path_maps
                .get(group.first().expect("size at least 2"))
                .expect("known")
                .to
                .clone();

            if let Some(idx) = name.rfind('.') {
                let (prefix, _) = name.split_at(idx);
                name = String::from(prefix);
            }
            self.define_group(&name, group);
        }
    }

    // final consumption of self to build the graph
    pub fn build(self) -> Graph {
        self.graph
    }

    fn ensure_link_node(&mut self, path: &Path) -> Option<LinkNode> {
        let full_location = match self.placement_maps.get(path) {
            Some(location) => location,
            None => {
                let mapped_name = match self.path_maps.get(path) {
                    Some(mapping) => mapping.to.clone(),
                    None => {
                        error!("Unexpected missing mapping for {:?}", path);
                        return None;
                    }
                };

                // have to create a stand-alone group
                self.define_group(&mapped_name, [path]);
                self.placement_maps.get(path).expect("just created a group")
            }
        };

        if self.zoomed_ids.contains(&full_location.group_id) {
            Some(full_location.clone())
        } else {
            Some(LinkNode {
                group_id: full_location.group_id.clone(),
                node_id: None,
            })
        }
    }

    pub fn add_link(&mut self, from: &Path, to: &Path) {
        let from = match self.ensure_link_node(from) {
            Some(p) => p,
            None => {
                debug!("NOT MAPPED: {:?}", from);
                return;
            }
        };

        let to = match self.ensure_link_node(to) {
            Some(p) => p,
            None => {
                debug!("NOT MAPPED: {:?}", to);
                return;
            }
        };

        self.graph.links.insert(GraphLink { from, to });
    }

    pub fn add_groups_from_gn(&mut self, gn_groups: Vec<GnTarget>) {
        for target in gn_groups {
            let items = target
                .sources
                .into_iter()
                .filter(|p| self.known_path(p))
                .collect::<Vec<_>>();
            if !items.is_empty() {
                self.define_group(&target.name, items);
            }
        }
    }

    pub fn define_group<T, P>(&mut self, group_name: &str, items: T)
    where
        T: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        if self.group_name_to_id.contains_key(group_name) {
            error!("Group {:?} already exists", group_name);
            return;
        }

        let mut g = Group {
            name: group_name.into(),
            zoomed: false,
            nodes: HashSet::default(),
        };
        let group_id = uuid::Uuid::now_v6(&[1, 0, 0, 0, 0, 0]).to_string();

        for path in items {
            let path = path.as_ref();
            if let Some(placement) = self.placement_maps.get(path) {
                let duplicate_pos = self
                    .graph
                    .groups
                    .get(&placement.group_id)
                    .map(|g| g.name.clone())
                    .unwrap_or_else(|| format!("ID: {}", placement.group_id));
                error!(
                    "{:?} in both: {:?} and {:?}",
                    path, group_name, duplicate_pos
                );
                continue;
            }

            let m = match self.path_maps.get(path) {
                Some(m) => m,
                None => {
                    error!("{:?} is missing a mapping", path);
                    continue;
                }
            };

            let node_id = uuid::Uuid::now_v6(&[0, 0, 0, 0, 0, g.nodes.len() as u8]).to_string();
            g.nodes.insert(MappedNode {
                id: node_id.clone(),
                path: PathBuf::from(path),
                display_name: m.to.clone(),
            });

            self.placement_maps.insert(
                PathBuf::from(path),
                LinkNode {
                    group_id: group_id.clone(),
                    node_id: Some(node_id),
                },
            );
        }

        self.group_name_to_id
            .insert(group_name.into(), group_id.clone());
        self.graph.groups.insert(group_id, g);
    }

    pub fn zoom_in(&mut self, group: &str) {
        let id = match self.group_name_to_id.get(group) {
            Some(id) => id,
            None => {
                error!("Group {:?} was NOT found", group);
                return;
            }
        };

        self.zoomed_ids.insert(id.clone());

        match self.graph.groups.get_mut(id) {
            Some(value) => value.zoomed = true,
            None => {
                error!(
                    "Internal error group {:?} with id {:?} was NOT found",
                    group, id
                );
            }
        }
    }
}
