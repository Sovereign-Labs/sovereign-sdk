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
}

impl From<Prefix> for sov_state::Prefix {
    fn from(prefix: Prefix) -> Self {
        let mut combined_prefix = Vec::with_capacity(
            prefix.module_path.len()
                + prefix.module_name.len()
                + prefix.storage_name.len()
                + 3 * DOMAIN_SEPARATOR.len(),
        );

        // We call this logic only once per module instantiation, so we don't have to use AlignedVec here.
        combined_prefix.extend(prefix.module_path.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix.extend(prefix.module_name.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix.extend(prefix.storage_name.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        sov_state::Prefix::new(combined_prefix)
    }
}
