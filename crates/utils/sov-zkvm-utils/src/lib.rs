use std::process::Command;

use convert_case::{Case, Casing};

/// Helper function that checks that value of the environment variable `SKIP_GUEST_BUILD` is used correctly
pub fn should_skip_guest_build(zk_vm: &str) -> bool {
    match std::env::var("SKIP_GUEST_BUILD")
        .as_ref()
        .map(|arg0: &String| String::as_str(arg0))
    {
        Ok("1") | Ok("true") => true,
        Ok(vm) if vm == zk_vm => true,
        Ok("0") | Ok("false") | Ok(_) | Err(_) => false,
    }
}

/// Output of rustc version comparison between native and ZKVM.
/// In case of mismatch both versions are returned, so the caller can have a meaningful error message.
pub enum RustComparisonResult {
    Same,
    Different {
        native_version: String,
        zkvm_version: String,
    },
}

/// Returns true if a native version of rustc matches ZKVM
pub fn does_rustc_match(zk_vm: &str) -> anyhow::Result<RustComparisonResult> {
    let risc0_cmd_output = Command::new("rustc")
        .env("RUSTUP_TOOLCHAIN", zk_vm)
        .arg("--version")
        .output()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;

    if !risc0_cmd_output.status.success() {
        anyhow::bail!(
            "Failed to get {} rustc version. Output: {:?}",
            zk_vm,
            risc0_cmd_output
        );
    }

    let native_version_cmd = Command::new("rustc")
        .arg("--version")
        .output()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    if !native_version_cmd.status.success() {
        anyhow::bail!(
            "Failed to get native cargo version: {:?}",
            native_version_cmd
        );
    }

    let zkvm_rustc_version =
        parse_version_string(&String::from_utf8_lossy(&risc0_cmd_output.stdout))?;
    let native_rustc_version =
        parse_version_string(&String::from_utf8_lossy(&native_version_cmd.stdout))?;
    if zkvm_rustc_version == native_rustc_version {
        Ok(RustComparisonResult::Same)
    } else {
        Ok(RustComparisonResult::Different {
            native_version: native_rustc_version,
            zkvm_version: zkvm_rustc_version,
        })
    }
}

fn parse_version_string(string: &str) -> anyhow::Result<String> {
    let version = string
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse version string"))?
        .split('-')
        .next()
        .unwrap();
    Ok(version.to_string())
}

/// Collects features that can be passed to guest ZKVM.
/// Checks all enabled features and passes all that are marked as forward.
/// If there's a feature enabled that is none in any of those lists, it will panic.
pub fn collect_features(features_to_forward: &[&str], features_to_skip: &[&str]) -> Vec<String> {
    // Cargo propagates activated features via environment variables:
    // any enabled feature named foo-bar becomes an env var CARGO_FEATURE_FOO_BAR=1,
    // available in build.rs via env::var("CARGO_FEATURE_FOO_BAR")
    std::env::vars()
        // First just take all enabled cargo features
        .filter_map(|(key, v)| {
            if v == "1" {
                key.strip_prefix("CARGO_FEATURE_").map(String::from)
            } else {
                None
            }
        })
        .filter_map(|enabled_feature_screaming| {
            let forward_match = features_to_forward
                .iter()
                .find(|&&f| f.to_case(Case::ScreamingSnake) == enabled_feature_screaming);
            let skip_match = features_to_skip
                .iter()
                .find(|&&f| f.to_case(Case::ScreamingSnake) == enabled_feature_screaming);

            match (forward_match, skip_match) {
                (Some(&forward), Some(&skip)) => {
                    panic!(
                        "Misconfiguration: enabled feature `{enabled_feature_screaming}` matches both a forward-rule ('{forward}') and a skip-rule ('{skip}'). Please resolve this ambiguity in the build script.",
                    );
                }
                (Some(&fwd), None) => Some(fwd.to_string()),
                (None, Some(_)) => None,
                (None, None) => {
                    panic!(
                        "An enabled feature (from env var `CARGO_FEATURE_{enabled_feature_screaming}`) is not specified in `features_to_forward` or `features_to_skip` in the build script. Please add `{}` to one of the lists.",
                        enabled_feature_screaming.to_case(Case::Kebab)
                    );
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    const TEST_ZK_VM: &str = "test_vm";

    #[test]
    fn test_skip_build_not_set() {
        env::remove_var("SKIP_GUEST_BUILD");
        assert!(!should_skip_guest_build(TEST_ZK_VM));
    }

    #[test]
    fn test_skip_build_set_to_1() {
        env::set_var("SKIP_GUEST_BUILD", "1");
        assert!(should_skip_guest_build(TEST_ZK_VM));
    }

    #[test]
    fn test_skip_build_set_to_true() {
        env::set_var("SKIP_GUEST_BUILD", "true");
        assert!(should_skip_guest_build(TEST_ZK_VM));
    }

    #[test]
    fn test_skip_build_set_to_zk_vm() {
        env::set_var("SKIP_GUEST_BUILD", TEST_ZK_VM);
        assert!(should_skip_guest_build(TEST_ZK_VM));
    }

    #[test]
    fn test_skip_build_set_to_0() {
        env::set_var("SKIP_GUEST_BUILD", "0");
        assert!(!should_skip_guest_build(TEST_ZK_VM));
    }

    #[test]
    fn test_skip_build_set_to_false() {
        env::set_var("SKIP_GUEST_BUILD", "false");
        assert!(!should_skip_guest_build(TEST_ZK_VM));
    }

    #[test]
    fn test_skip_build_set_to_other_string() {
        env::set_var("SKIP_GUEST_BUILD", "some_other_value");
        assert!(!should_skip_guest_build(TEST_ZK_VM));
    }

    #[test]
    fn test_skip_build_set_to_different_zk_vm() {
        env::set_var("SKIP_GUEST_BUILD", "another_vm");
        assert!(!should_skip_guest_build(TEST_ZK_VM));
    }
}
