use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub enum FileGroupType {
    Manual,
    HeaderSource,
    Existing,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub enum Node {
    Single {
        name: String,
    },
    Group {
        title: String,
        files: Vec<String>,
        group_type: FileGroupType,
    },
}

/// Represents dependency management
pub trait DependencyGraph {
    /// mark a dependency from src to dest
    fn add_dependency(&mut self, src: String, dest: String);
}

#[derive(Default, Debug)]
pub struct DependencyGroups {
    nodes: HashSet<Node>,
    dependencies: HashMap<String, HashSet<String>>, // what node depends on what node
}

impl DependencyGroups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, n: Node) {
        self.nodes.insert(n);
    }
}

impl DependencyGraph for DependencyGroups {
    fn add_dependency(&mut self, src: String, dest: String) {
        if let Some(existing) = self.dependencies.get_mut(&src) {
            existing.insert(dest);
        } else {
            self.dependencies.insert(src, {
                let mut s = HashSet::new();
                s.insert(dest);
                s
            });
        }
    }
}
