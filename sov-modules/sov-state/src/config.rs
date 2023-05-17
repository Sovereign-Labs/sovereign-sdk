use std::path::PathBuf;

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Path to folder where storage files will be stored
    pub path: PathBuf,
}
