use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::Serialize;
use tera::{Context, Tera};
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};
use tracing::{debug, error};

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

    /// If this groop is zoomed in
    pub zoomed: bool,

    // some color name to use
    pub color: String,

    /// what are the nodes
    pub nodes: HashSet<MappedNode>,
}

impl Group {
    /// re-creates a new version of the group with all unique IDs changed new
    ///
    /// returns a brand new unique id for the group as well as a remade version
    pub fn zoomed(&self, id_map: &mut HashMap<String, String>) -> Self {
        let mut nodes = HashSet::new();

        for n in self.nodes.iter() {
            let new_id = format!("z{}", n.id);
            nodes.insert(MappedNode {
                id: new_id.clone(),
                path: n.path.clone(),
                display_name: n.display_name.clone(),
            });
            id_map.insert(n.id.clone(), new_id);
        }
        Self {
            name: format!("{} (ZOOM)", self.name),
            zoomed: true,
            color: self.color.clone(), // TODO: nicer colors?
            nodes,
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Serialize)]
pub struct LinkNode {
    pub group_id: String,
    pub node_id: Option<String>,
}

impl LinkNode {
    pub fn without_node(&self) -> LinkNode {
        if self.node_id.is_none() {
            self.clone()
        } else {
            LinkNode {
                group_id: self.group_id.clone(),
                node_id: None,
            }
        }
    }

    pub fn try_remap(&self, m: &HashMap<String, String>) -> Option<Self> {
        let node_id = match self.node_id {
            Some(ref id) => Some(m.get(id)?.clone()),
            None => None,
        };

        Some(Self {
            group_id: m.get(&self.group_id)?.clone(),
            node_id,
        })
    }
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Serialize)]
pub struct GraphLink {
    pub from: LinkNode,
    pub to: LinkNode,
}

impl GraphLink {
    pub fn try_remap(&self, m: &HashMap<String, String>) -> Option<Self> {
        Some(Self {
            from: self.from.try_remap(m)?,
            to: self.to.try_remap(m)?,
        })
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Graph {
    groups: HashMap<String, Group>,
    links: HashSet<GraphLink>,
    zoomed: HashSet<String>,
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

        writer
            .write(
                tera.render(
                    "dot_template",
                    &Context::from_serialize(self).map_err(Error::RenderError)?,
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

    /// What graphs are focused zoomed. Remove links that span non-focused
    focus_zoomed: HashSet<String>,
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
            self.define_group(&name, "aqua", group);
        }
    }

    // final consumption of self to build the graph
    pub fn build(mut self) -> Graph {
        // Group all items without links
        let known_placement = self.placement_maps.keys().collect::<HashSet<_>>();

        // create a single group of all node-ids that have no links ... to see stand-alone items
        let no_link_nodes = self
            .path_maps
            .keys()
            .filter(|k| !known_placement.contains(*k)).cloned()
            .collect::<Vec<_>>();

        if !no_link_nodes.is_empty() {
            self.define_group("NO DEPENDENCIES OR GROUPS", "gray85", no_link_nodes);
        }

        // figure out zoomed items;
        let mut link_map = HashMap::new();

        let mut new_groups = Vec::new();

        let mut zoom_colors = [
            "powderblue",
            "peachpuff",
            "thistle",
            "honeydew",
            "khaki",
            "lavender",
        ]
        .iter()
        .cycle();

        for (id, group) in self
            .graph
            .groups
            .iter()
            .filter(|(id, _)| self.graph.zoomed.contains(*id))
        {
            let new_id = format!("z{}", id);
            link_map.insert(id.clone(), new_id.clone());
            new_groups.push((new_id, {
                let mut z = group.zoomed(&mut link_map);
                z.color = zoom_colors.next().expect("infinite").to_string();
                z
            }));
        }
        // zoom changed now
        self.graph.zoomed = new_groups.iter().map(|(id, _)| id.clone()).collect();
        self.graph.groups.extend(new_groups);

        let zoom_links = self
            .graph
            .links
            .iter()
            .filter(|l| {
                link_map.contains_key(&l.from.group_id) && link_map.contains_key(&l.to.group_id)
            })
            .filter_map(|l| {
                if !(self.focus_zoomed.is_empty()
                    || l.from.group_id == l.to.group_id
                    || self.focus_zoomed.contains(&l.from.group_id)
                    || self.focus_zoomed.contains(&l.to.group_id))
                {
                    return None;
                }

                let l = l.try_remap(&link_map);
                if l.is_none() {
                    error!("FAILED TO REMAP: {:?}", l);
                }
                l
            })
            .collect::<HashSet<_>>();

        // Create group links only here
        let links = self
            .graph
            .links
            .iter()
            .map(|l| GraphLink {
                from: l.from.without_node(),
                to: l.to.without_node(),
            })
            .filter(|l| l.from != l.to)
            .collect::<HashSet<_>>();

        self.graph.links = {
            let mut v = HashSet::new();
            v.extend(links);
            v.extend(zoom_links);
            v
        };
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
                self.define_group(&mapped_name, "thistle", [path]);
                self.placement_maps.get(path).expect("just created a group")
            }
        };

        Some(full_location.clone())
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

        if from == to {
            return;
        }

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
                self.define_group(&target.name, "lightgreen", items);
            }
        }
    }

    pub fn define_group<T, P>(&mut self, group_name: &str, color: &str, items: T)
    where
        T: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        if self.group_name_to_id.contains_key(group_name) {
            error!("Group {:?} already exists", group_name);
            return;
        }

        let mut g = Group {
            zoomed: false,
            name: group_name.into(),
            color: color.into(),
            nodes: HashSet::default(),
        };
        let group_id = format!("grp_{}", uuid::Uuid::now_v6(&[1, 0, 0, 0, 0, 0]))
            .to_string()
            .replace('-', "_");

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
                    // Generally this means someone created a `manual group` however source file was not
                    // loaded, for example loading sources from compile_database but not all files are compiled
                    // by this build run
                    error!("{:?} is a source file without a map entry. Cannot add to group (is this a loaded source file?).", path);
                    continue;
                }
            };

            let node_id = format!(
                "node_{}",
                uuid::Uuid::now_v6(&[0, 0, 0, 0, 0, g.nodes.len() as u8])
            )
            .to_string()
            .replace('-', "_");
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

        if g.nodes.is_empty() {
            error!("Group {:?} is empty. Will not create.", group_name);
            return;
        }

        self.group_name_to_id
            .insert(group_name.into(), group_id.clone());
        self.graph.groups.insert(group_id, g);
    }

    pub fn zoom_in(&mut self, group: &str, focused: bool) {
        let id = match self.group_name_to_id.get(group) {
            Some(id) => id,
            None => {
                error!("Group {:?} was NOT found", group);
                return;
            }
        };

        self.graph.zoomed.insert(id.clone());
        if focused {
            self.focus_zoomed.insert(id.clone());
        }
    }
}
