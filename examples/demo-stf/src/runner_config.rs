use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::de::DeserializeOwned;
pub use sov_modules_api::default_context::DefaultContext;
pub use sov_state::config::Config as StorageConfig;

pub fn from_toml_path<P: AsRef<Path>, R: DeserializeOwned>(path: P) -> anyhow::Result<R> {
    let mut contents = String::new();
    {
        let mut file = File::open(path)?;
        file.read_to_string(&mut contents)?;
    }

    let result: R = toml::from_str(&contents)?;

    Ok(result)
}

#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub storage: StorageConfig,
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use tempfile::{tempdir, NamedTempFile};

    use super::*;

    fn create_config_from(content: &str) -> NamedTempFile {
        let mut config_file = NamedTempFile::new().unwrap();
        config_file.write_all(content.as_bytes()).unwrap();
        config_file
    }

    #[test]
    fn test_correct_config() {
        let config = r#"
            [storage]
            path = "/tmp"
        "#;

        let config_file = create_config_from(config);

        let config: Config = from_toml_path(config_file.path()).unwrap();
        let expected = Config {
            storage: StorageConfig {
                path: PathBuf::from("/tmp"),
            },
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn test_incorrect_path() {
        // Not closed quote
        let config = r#"
            [storage]
            path = "/tmp
        "#;
        let config_file = create_config_from(config);

        let config: anyhow::Result<Config> = from_toml_path(config_file.path());

        assert!(config.is_err());
        let error = config.unwrap_err().to_string();
        let expected_error = format!(
            "{}{}{}",
            "TOML parse error at line 3, column 25\n  |\n3 |",
            "             path = \"/tmp\n  |                         ^\n",
            "invalid basic string\n"
        );
        assert_eq!(error, expected_error);
    }
    //
    #[test]
    fn test_non_existent_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("non_existing_config.toml");

        let config: anyhow::Result<Config> = from_toml_path(path);

        assert!(config.is_err());
        assert!(config.unwrap_err().to_string().ends_with("(os error 2)"));
    }
}
