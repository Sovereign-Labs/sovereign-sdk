use std::collections::HashMap;

use serde::Serialize;

use super::{Prefix, StateItemInfo};
use crate::{ModuleId, ModuleInfo};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum StateItemContents<K, V> {
    Value { value: Option<V> },
    Vec { length: u64 },
    VecElement { index: u64, value: V },
    MapElement { key: K, value: V },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", rename = "module")]
pub struct ModuleObject {
    pub id: ModuleId,
    pub name: String,
    pub description: Option<String>,
    pub prefix: Prefix,
    pub state_items: HashMap<String, StateItemInfo>,
}

impl ModuleObject {
    pub fn new(
        module: &(impl ModuleInfo + ?Sized),
        description: Option<String>,
        state_items: HashMap<String, StateItemInfo>,
    ) -> Self {
        Self {
            id: *module.id(),
            description,
            name: module.prefix().module_name().to_owned(),
            prefix: Prefix(module.prefix().into()),
            state_items,
        }
    }
}
