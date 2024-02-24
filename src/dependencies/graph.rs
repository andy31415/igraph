use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use tracing::{error, warn};

use super::path_mapper::{PathMapper, PathMapping};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
#[derive(Debug, PartialEq)]
pub struct Group {
    /// nice display name
    pub name: String,

    /// are the nodes expanded out
    pub zoomed: bool,

    /// what are the nodes
    pub nodes: HashSet<MappedNode>,
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphLink {
    pub group_id: String,
    pub node_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub groups: HashMap<String, Group>,
    pub links: HashSet<GraphLink>,
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
    placement_maps: HashMap<PathBuf, GraphLink>,
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

    pub fn define_group<T>(&mut self, group_name: &str, items: T)
    where
        T: Iterator<Item = PathBuf>,
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
            if let Some(placement) = self.placement_maps.get(&path) {
                error!("{:?} already placed in {:?}", path, placement.group_id);
                continue;
            }

            let m = match self.path_maps.get(&path) {
                Some(m) => m,
                None => {
                    error!("{:?} is missing a mapping", path);
                    continue;
                }
            };

            g.nodes.insert(MappedNode {
                id: uuid::Uuid::now_v6(&[0, 0, 0, 0, 0, g.nodes.len() as u8]).to_string(),
                path,
                display_name: m.to.clone(),
            });
        }

        self.group_name_to_id
            .insert(group_name.into(), group_id.clone());
        self.graph.groups.insert(group_id, g);
    }

    pub fn zoom_in(&mut self, group: &str) {
        match self.graph.groups.get_mut(group) {
            Some(value) => value.zoomed = true,
            None => error!("Group {:?} was NOT found", group),
        }
    }
}
