//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use std::sync::{Arc, Mutex};

use crate::guest::SP1Guest;
use crate::SP1MethodId;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::aggregated_proof::common::{
    AggregatedProofWitness, DeferredProofInput, PreviousOuterProofWitness, SerializedPubValues,
};
use sov_rollup_interface::zk::aggregated_proof::{
    BlockHeaderWithProof, BlockProof, CodeCommitmentHash, OuterZkvmHost, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{SerializedZkProof, ZkvmHost};
use sp1_sdk::blocking::ProveRequest;
use sp1_sdk::blocking::{EnvProver, EnvProvingKey, Prover, ProverClient};
use sp1_sdk::ProvingKey;
use sp1_sdk::SP1VerifyingKey;
use sp1_sdk::{HashableKey, SP1Proof, SP1Stdin};

/// SP1 host that produces aggregated (outer) proofs by recursively verifying
/// a batch of inner state-transition proofs inside an SP1 guest program.
///
/// Each call to [`run`](Self::run) generates a compressed aggregation proof
/// that covers one batch of inner proofs.  When called multiple times the host
/// automatically chains proofs: the previous aggregation proof is fed back as
/// a deferred proof input so the guest can verify continuity.
#[derive(Clone)]
pub struct SP1AggregationHost {
    inner: Arc<Inner>,
}

struct Inner {
    prover: SP1Prover,
    outer_vk: sp1_sdk::SP1VerifyingKey,
    inner_vk: sp1_sdk::SP1VerifyingKey,
    prev_agg_proof: Mutex<Option<sp1_sdk::SP1ProofWithPublicValues>>,
}

impl SP1AggregationHost {
    /// Creates a new aggregation host from the aggregation guest `elf` binary
    /// and the verifying key (`inner_method_id`) of the inner proof program.
    pub fn new(elf: &'static [u8], inner_vk: sp1_sdk::SP1VerifyingKey) -> anyhow::Result<Self> {
        let prover = SP1Prover::new(elf)?;
        let outer_vk = prover.verifying_key().clone();

        Ok(Self {
            inner: Arc::new(Inner {
                prover,
                outer_vk,
                inner_vk,
                prev_agg_proof: Mutex::new(None),
            }),
        })
    }

    /// Returns the code commitment (verifying key) of the aggregation program.
    pub fn code_commitment(&self) -> SP1MethodId {
        SP1MethodId(self.inner.outer_vk.hash_u32())
    }

    /// Restores the latest accepted aggregated proof so the next aggregation
    /// can continue the recursive chain after a restart.
    pub fn restore_persisted_aggregated_proof(
        &self,
        aggregated_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<()> {
        let aggregated_proof = crate::decode_sp1_proof(&aggregated_proof.to_serialized_zk_proof())?;
        let mut prev_agg_proof = self
            .inner
            .prev_agg_proof
            .lock()
            .map_err(|e| anyhow::anyhow!("prev_agg_proof mutex poisoned: {e}"))?;
        *prev_agg_proof = Some(aggregated_proof);
        Ok(())
    }

    /// Generates a compressed aggregation proof over the supplied inner
    /// `proofs_and_headers`.  If a previous aggregation proof exists it is
    /// included as a deferred proof input for recursive verification.
    pub fn run<Da: DaSpec>(
        &self,
        proofs_and_headers: Vec<BlockHeaderWithProof<Da>>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        anyhow::ensure!(
            !proofs_and_headers.is_empty(),
            "At least one inner proof is required"
        );

        let mut stdin = SP1Stdin::new();

        let mut prev_agg_proof = self
            .inner
            .prev_agg_proof
            .lock()
            .map_err(|e| anyhow::anyhow!("prev_agg_proof mutex poisoned: {e}"))?;

        let prev_outer_proof_witness = if let Some(previous_outer_proof) = prev_agg_proof.as_ref() {
            let serialized = crate::encode_sp1_proof(previous_outer_proof)?;
            let public_values = self.inner.prover.add_proof_helper(
                &mut stdin,
                &serialized,
                &self.inner.outer_vk,
            )?;

            Some(PreviousOuterProofWitness { public_values })
        } else {
            None
        };

        let mut proof_inputs = Vec::with_capacity(proofs_and_headers.len());
        for proof_and_header in proofs_and_headers {
            let public_values = self.inner.prover.add_proof_helper(
                &mut stdin,
                &proof_and_header.proof,
                &self.inner.inner_vk,
            )?;
            let proof_input = DeferredProofInput::<Da> {
                public_values,
                da_block_header: proof_and_header.da_block_header,
            };

            proof_inputs.push(proof_input);
        }

        let outer_vk_hash = self.inner.outer_vk.hash_u32();
        let inner_vk = &self.inner.inner_vk;

        let inner_vkey_hash = CodeCommitmentHash::from_u32_array(inner_vk.hash_u32());
        let outer_vkey_hash = CodeCommitmentHash::from_u32_array(outer_vk_hash);

        let witness = AggregatedProofWitness {
            proof_inputs,
            inner_vkey_hash,
            outer_vkey_hash,
            prev_outer_proof_witness,
        };

        stdin.write(&witness);

        let agg_proof = self.inner.prover.run(stdin)?;
        let serialized = crate::encode_sp1_proof(&agg_proof)?;
        *prev_agg_proof = Some(agg_proof);

        Ok(SerializedAggregatedProof {
            raw_aggregated_proof: serialized.raw_proof,
        })
    }
}

/// SP1 prover bundling the [`EnvProver`] with its proving key.
#[derive(Clone)]
pub struct SP1Prover {
    prover: EnvProver,
    pk: Arc<EnvProvingKey>,
}

impl SP1Prover {
    /// Create a new [`SP1Prover`] by setting up a proving key from `elf`.
    pub fn new(elf: &[u8]) -> anyhow::Result<Self> {
        let (prover, pk) = prover_and_pk(elf)?;
        Ok(Self {
            prover,
            pk: Arc::new(pk),
        })
    }

    /// Verification key.
    pub fn verifying_key(&self) -> &SP1VerifyingKey {
        self.pk.verifying_key()
    }

    /// Method id.
    pub fn method_id(&self) -> SP1MethodId {
        SP1MethodId(self.pk.verifying_key().hash_u32())
    }

    fn add_proof_helper(
        &self,
        stdin: &mut SP1Stdin,
        proof: &SerializedZkProof,
        vk: &sp1_sdk::SP1VerifyingKey,
    ) -> anyhow::Result<SerializedPubValues> {
        let proof = crate::decode_sp1_proof(proof)?;

        let SP1Proof::Compressed(recursion_proof) = &proof.proof else {
            anyhow::bail!("Expected a compressed SP1 proof");
        };

        stdin.write_proof((**recursion_proof).clone(), vk.vk.clone());
        Ok(SerializedPubValues {
            pub_values: proof.public_values.to_vec(),
        })
    }

    fn run(&self, stdin: SP1Stdin) -> anyhow::Result<sp1_sdk::SP1ProofWithPublicValues> {
        // Under the mock backend the inner compressed proofs are dummies that
        // would fail the executor-side deferred-proof check. Skip that check so
        // mock aggregation can run end-to-end; real backends keep it on.
        let request = self.prover.prove(&self.pk, stdin).compressed();

        let is_mock = matches!(&self.prover, &EnvProver::Mock(_));
        let request = if is_mock {
            request.deferred_proof_verification(false)
        } else {
            request
        };
        let output: sp1_sdk::SP1ProofWithPublicValues = request
            .run()
            .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;

        Ok(output)
    }

    /// Serialize `item` as a hint, produce a compressed proof, and encode it.
    pub fn add_hint_and_run<T: Serialize>(&self, item: &T) -> anyhow::Result<SerializedZkProof> {
        let mut stdin = SP1Stdin::new();
        stdin.write(item);
        let output = self.run(stdin)?;
        crate::encode_sp1_proof(&output)
    }
}

/// SP1 Host implementation.
#[derive(Clone)]
pub struct SP1Host {
    prover: SP1Prover,
    outer_vk: Arc<SP1VerifyingKey>,
}

/// Instantiate a new SP1 Host.
impl SP1Host {
    /// Create a new SP1 Host backed by a real proving key derived from `elf`.
    pub fn new(elf: &[u8], outer_vk: Arc<SP1VerifyingKey>) -> anyhow::Result<Self> {
        Ok(Self {
            prover: SP1Prover::new(elf)?,
            outer_vk,
        })
    }

    fn add_hint_deferred_and_run_helper<T: Serialize>(
        &mut self,
        item: &T,
        agg_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<SerializedZkProof> {
        let mut stdin = SP1Stdin::new();

        for raw_agg_proof in agg_proofs {
            self.prover.add_proof_helper(
                &mut stdin,
                &raw_agg_proof.to_serialized_zk_proof(),
                &self.outer_vk,
            )?;
        }

        stdin.write(item);
        let output = self.prover.run(stdin)?;
        crate::encode_sp1_proof(&output)
    }

    /// Verification key.
    pub fn verifying_key(&self) -> &SP1VerifyingKey {
        self.prover.verifying_key()
    }
}

/// Verification key.
pub fn verifying_key_from_elf(elf: &[u8]) -> anyhow::Result<SP1VerifyingKey> {
    let (_, pk) = prover_and_pk(elf)?;
    Ok(pk.verifying_key().clone())
}

/// Computes the [`SP1MethodId`] (code commitment) for the given guest ELF.
pub fn code_commitment_from_elf(elf: &[u8]) -> anyhow::Result<SP1MethodId> {
    let vk = verifying_key_from_elf(elf)?;
    Ok(SP1MethodId(vk.hash_u32()))
}

fn prover_and_pk(elf: &[u8]) -> anyhow::Result<(EnvProver, EnvProvingKey)> {
    let prover = ProverClient::from_env();

    let pk = prover
        .setup(elf.into())
        .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;

    Ok((prover, pk))
}

impl core::fmt::Debug for SP1Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Host").finish()
    }
}

impl ZkvmHost for SP1Host {
    type Guest = SP1Guest;

    fn add_hint_deferred_and_run<T: Serialize>(
        &mut self,
        item: &T,
        agg_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<SerializedZkProof> {
        self.add_hint_deferred_and_run_helper(item, agg_proofs)
    }

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(self.prover.method_id())
    }
}

impl OuterZkvmHost for SP1AggregationHost {
    fn run_proof_aggregation<Address: Serialize + Clone, Da: DaSpec, Root: Serialize + Clone>(
        &self,
        genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        let has_prev_agg_proof = self
            .inner
            .prev_agg_proof
            .lock()
            .map_err(|e| anyhow::anyhow!("prev_agg_proof mutex poisoned: {e}"))?
            .is_some();

        if !has_prev_agg_proof {
            let first_block_proof = headers_with_block_proofs
                .first()
                .map(|(_, proof)| proof)
                .expect("create_aggregated_proof requires at least one block proof");

            anyhow::ensure!(
                serialize_for_continuity_check(&first_block_proof.st.initial_state_root)?
                    == serialize_for_continuity_check(&genesis_state_root)?,
                "Missing previous SP1 aggregated proof: this batch starts after genesis and would reset outer-proof continuity"
            );
        }

        let proofs_and_headers: Vec<BlockHeaderWithProof<Da>> = headers_with_block_proofs
            .into_iter()
            .map(|(header, proof)| BlockHeaderWithProof {
                da_block_header: header,
                proof: proof.proof,
            })
            .collect();

        self.run(proofs_and_headers)
    }

    fn restore_persisted_aggregated_proof(
        &self,
        aggregated_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<()> {
        SP1AggregationHost::restore_persisted_aggregated_proof(self, aggregated_proof)
    }
}

fn serialize_for_continuity_check<T: Serialize>(item: &T) -> anyhow::Result<Vec<u8>> {
    bincode::serialize(item).map_err(Into::into)
}
