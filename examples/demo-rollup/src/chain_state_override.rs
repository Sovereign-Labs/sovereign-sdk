//! Utility for overriding code commitment fields in a `chain_state.json` genesis file.

use std::path::Path;

use anyhow::Context as _;
use sov_sp1_adapter::SP1MethodId;

/// Overrides `inner_code_commitment` and `outer_code_commitment` in the given
/// `chain_state.json` file. All other fields are preserved.
pub fn override_code_commitments_in_chain_state(
    chain_state_path: &Path,
    inner: &SP1MethodId,
    outer: &SP1MethodId,
) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(chain_state_path).with_context(|| {
        format!(
            "Failed to read chain_state.json at {}",
            chain_state_path.display()
        )
    })?;

    let mut value: serde_json::Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "Failed to parse chain_state.json at {}",
            chain_state_path.display()
        )
    })?;

    let obj = value.as_object_mut().with_context(|| {
        format!(
            "chain_state.json at {} is not a JSON object",
            chain_state_path.display()
        )
    })?;

    obj.insert(
        "inner_code_commitment".to_string(),
        serde_json::to_value(inner.0)?,
    );
    obj.insert(
        "outer_code_commitment".to_string(),
        serde_json::to_value(outer.0)?,
    );

    let serialized = serde_json::to_string_pretty(&value)?;
    std::fs::write(chain_state_path, serialized).with_context(|| {
        format!(
            "Failed to write chain_state.json at {}",
            chain_state_path.display()
        )
    })?;

    Ok(())
}
