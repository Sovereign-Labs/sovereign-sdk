use crate::{Context, Hasher};

// separator == "/"
const DOMAIN_SEPARATOR: [u8; 1] = [47];

/// A unique identifier for each state variable in a module.
#[derive(Debug, PartialEq, Eq)]
pub struct Prefix {
    module_path: &'static str,
    module_name: &'static str,
    storage_name: &'static str,
}

impl Prefix {
    pub fn new(
        module_path: &'static str,
        module_name: &'static str,
        storage_name: &'static str,
    ) -> Self {
        Self {
            module_path,
            module_name,
            storage_name,
        }
    }

    fn combine_prefix(&self) -> Vec<u8> {
        let mut combined_prefix = Vec::with_capacity(
            self.module_path.len()
                + self.module_name.len()
                + self.storage_name.len()
                + 3 * DOMAIN_SEPARATOR.len(),
        );

        combined_prefix.extend(self.module_path.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix.extend(self.module_name.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix.extend(self.storage_name.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix
    }

    pub fn hash<C: Context>(&self) -> [u8; 32] {
        let combined_prefix = self.combine_prefix();
        C::Hasher::hash(&combined_prefix)
    }
}

impl From<Prefix> for sov_state::Prefix {
    fn from(prefix: Prefix) -> Self {
        let combined_prefix = prefix.combine_prefix();
        sov_state::Prefix::new(combined_prefix)
    }
}
