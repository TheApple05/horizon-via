// SPDX-License-Identifier: MIT

//! Host-side source-overlay adapter; not part of the standalone native library.
use horizon_bedrock::{blocks::HostRegistry, translation::HostBlockId};
use horizon_world::{chunk::BlockState, registry::BlockRegistry};

pub struct Registry<'a> {
    registry: &'a BlockRegistry,
}
impl<'a> Registry<'a> {
    pub fn new(registry: &'a BlockRegistry) -> Self {
        Self { registry }
    }
}
impl HostRegistry for Registry<'_> {
    fn resolve(&self, name: &str, properties: &[(&str, &str)]) -> Option<HostBlockId> {
        let mut complete = std::collections::BTreeMap::new();
        for &(key, value) in properties {
            if complete
                .insert(key.to_string(), value.to_string())
                .is_some()
            {
                return None;
            }
        }
        self.registry
            .id(&BlockState {
                name: name.to_string(),
                properties: complete.into(),
            })
            .map(|id| HostBlockId::from_registry_id(id.0))
    }
}
