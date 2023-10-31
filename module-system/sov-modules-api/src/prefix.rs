use sha2::Digest;
use sov_modules_core::Context;

// separator == "/"
const DOMAIN_SEPARATOR: [u8; 1] = [47];

/// A unique identifier for each state variable in a module.
#[derive(Debug, PartialEq, Eq)]
pub struct Prefix {
    module_path: &'static str,
    module_name: &'static str,
    storage_name: Option<&'static str>,
}

impl Prefix {
    pub fn new_storage(
        module_path: &'static str,
        module_name: &'static str,
        storage_name: &'static str,
    ) -> Self {
        Self {
            module_path,
            module_name,
            storage_name: Some(storage_name),
        }
    }

    pub fn new_module(module_path: &'static str, module_name: &'static str) -> Self {
        Self {
            module_path,
            module_name,
            storage_name: None,
        }
    }

    fn combine_prefix(&self) -> Vec<u8> {
        let storage_name_len = self
            .storage_name
            .map(|name| name.len() + DOMAIN_SEPARATOR.len())
            .unwrap_or_default();

        let mut combined_prefix = Vec::with_capacity(
            self.module_path.len()
                + self.module_name.len()
                + 2 * DOMAIN_SEPARATOR.len()
                + storage_name_len,
        );

        combined_prefix.extend(self.module_path.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix.extend(self.module_name.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        if let Some(storage_name) = self.storage_name {
            combined_prefix.extend(storage_name.as_bytes());
            combined_prefix.extend(DOMAIN_SEPARATOR);
        }
        combined_prefix
    }

    pub fn hash<C: Context>(&self) -> [u8; 32] {
        let combined_prefix = self.combine_prefix();
        let mut hasher = C::Hasher::new();
        hasher.update(combined_prefix);
        hasher.finalize().into()
    }
}

impl From<Prefix> for sov_state::Prefix {
    fn from(prefix: Prefix) -> Self {
        let combined_prefix = prefix.combine_prefix();
        sov_state::Prefix::new(combined_prefix)
    }
}
