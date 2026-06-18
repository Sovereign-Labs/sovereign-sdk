//! Light-client support for rollups running the SP1 zkVM.
use anyhow::Context as _;
use serde::de::DeserializeOwned;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregateProofVerifier, AggregatedProofPublicData, CodeCommitmentHash,
    SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{CodeCommitmentTrait, ZkLightClient};
use std::path::{Path, PathBuf};

use crate::host::code_commitment_from_elf;
use crate::{SP1MethodId, SP1Verifier};

/// Light client for a rollup running the SP1 zkVM.
///
/// Derives the outer verification key from the aggregation-circuit ("outer")
/// guest ELF on disk.
pub struct Sp1LightClient {
    /// The inner code commitment hash the rollup's state-transition proofs must use.
    expected_inner_vkey_hash: CodeCommitmentHash,
    /// The outer code commitment used to verify aggregated proofs.
    outer_code_commitment: SP1MethodId,
    /// Timeout-configured HTTP client used to fetch proofs from the node.
    http_client: reqwest::Client,
}

impl Sp1LightClient {
    /// Creates a new [`Sp1LightClient`] that derives its trusted inner and
    /// outer verification keys from local guest ELFs.
    pub async fn new(
        inner_elf_path: impl Into<PathBuf>,
        outer_elf_path: impl Into<PathBuf>,
    ) -> anyhow::Result<Self> {
        let inner_elf_path = inner_elf_path.into();
        let outer_elf_path = outer_elf_path.into();
        let (expected_inner_vkey_hash, outer_code_commitment) =
            tokio::task::spawn_blocking(move || {
                let inner_elf = read_guest_elf(&inner_elf_path, "inner")?;
                let outer_elf = read_guest_elf(&outer_elf_path, "outer")?;
                let expected_inner_vkey_hash = code_commitment_from_elf(&inner_elf)
                    .context("Failed to compute the verification key from the inner ELF")?
                    .to_hash();
                let outer_code_commitment = code_commitment_from_elf(&outer_elf)
                    .context("Failed to compute the verification key from the outer ELF")?;
                anyhow::Ok((expected_inner_vkey_hash, outer_code_commitment))
            })
            .await
            .context("Verification-key derivation task panicked")??;

        Ok(Self {
            expected_inner_vkey_hash,
            outer_code_commitment,
            http_client: Self::build_http_client(),
        })
    }
}

impl ZkLightClient for Sp1LightClient {
    fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    fn verify_aggregated_proof<Address, Da, Root>(
        &self,
        proof: SerializedAggregatedProof,
    ) -> anyhow::Result<AggregatedProofPublicData<Address, Da, Root>>
    where
        Address: DeserializeOwned,
        Da: DaSpec,
        Root: DeserializeOwned,
    {
        let expected_outer_vkey_hash = self.outer_code_commitment.to_hash();
        let public_data =
            AggregateProofVerifier::<SP1Verifier>::new(self.outer_code_commitment.clone())
                .verify(&proof)
                .context("Aggregated proof verification failed")?;

        anyhow::ensure!(
            public_data.outer_vk_hash == expected_outer_vkey_hash,
            "Aggregated proof outer code commitment mismatch: proof claims {}, light client expects {}",
            public_data.outer_vk_hash,
            expected_outer_vkey_hash
        );

        anyhow::ensure!(
            public_data.inner_vkey_hash == self.expected_inner_vkey_hash,
            "Aggregated proof inner code commitment mismatch: proof claims {}, light client expects {}",
            public_data.inner_vkey_hash,
            self.expected_inner_vkey_hash
        );

        Ok(public_data)
    }
}

/// Reads and validates a guest ELF at `path`.
fn read_guest_elf(path: &Path, circuit: &str) -> anyhow::Result<Vec<u8>> {
    let guest_elf = std::fs::read(path)
        .with_context(|| format!("Failed to read the {circuit} ELF at {}", path.display()))?;
    anyhow::ensure!(
        !guest_elf.is_empty(),
        "{} ELF at {} is empty",
        circuit,
        path.display()
    );
    Ok(guest_elf)
}
