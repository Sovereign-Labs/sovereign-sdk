use std::path::Path;

use anyhow::Context;
use celestia_types::namespace_data::NamespaceData;
use celestia_types::nmt::Namespace;
use celestia_types::ExtendedHeader;

use crate::header::CelestiaHeader;

pub struct BlockFixture {
    pub header: CelestiaHeader,
    pub batch_rows: NamespaceData,
    pub proof_rows: NamespaceData,
}

pub fn load_block_fixture(
    batch_namespace: Namespace,
    proof_namespace: Namespace,
    path: &Path,
) -> anyhow::Result<BlockFixture> {
    let header: ExtendedHeader = load_json(path, "header.json")?;
    let batch_rows: NamespaceData = load_json(path, "rollup_batch_rows.json")?;
    let proof_rows: NamespaceData = load_json(path, "rollup_proof_rows.json")?;
    ensure_namespace(&batch_rows, batch_namespace, "batch")?;
    ensure_namespace(&proof_rows, proof_namespace, "proof")?;
    Ok(BlockFixture {
        header: header.into(),
        batch_rows,
        proof_rows,
    })
}

fn ensure_namespace(rows: &NamespaceData, namespace: Namespace, label: &str) -> anyhow::Result<()> {
    for row in rows.rows() {
        for share in &row.shares {
            let found = share.namespace();
            anyhow::ensure!(
                found == namespace,
                "{label} fixture share is in {found:?}, expected {namespace:?}"
            );
        }
    }
    Ok(())
}

fn load_json<T: serde::de::DeserializeOwned>(path: &Path, name: &str) -> anyhow::Result<T> {
    let path = path.join(name);
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    serde_json::from_str(&contents).with_context(|| format!("Failed to parse {}", path.display()))
}
