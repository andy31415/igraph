use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct PathMapping {
    pub from: PathBuf,
    pub to: String,
}

/// Maps path buffers into actual strings
#[derive(Default, Debug, Clone)]
pub struct PathMapper {
    mappings: Vec<PathMapping>,
}

impl PathMapper {
    pub fn add_mapping(&mut self, mapping: PathMapping) {
        self.mappings.push(mapping);
    }

    pub fn try_invert(&self, p: &str) -> Option<PathBuf> {
        self.mappings
            .iter()
            .filter_map(|m| {
                p.strip_prefix(&m.to).map(|tail| {
                    let mut p = m.from.clone();
                    p.push(PathBuf::from(tail));
                    p
                })
            })
            .next()
    }

    /// Map the given input path into a final name string
    ///
    /// Returns the mapped String if a mapping exists, otherwise
    /// it returns None
    pub fn try_map(&self, p: &Path) -> Option<String> {
        for mapping in self.mappings.iter() {
            if let Ok(rest) = p.strip_prefix(&mapping.from) {
                return Some(mapping.to.clone() + &rest.to_string_lossy());
            }
        }
        None
    }
}
