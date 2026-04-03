//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use crate::guest::SP1Guest;
use crate::SP1MethodId;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::aggregated_proof::common::{
    AggregatedProofWitness, DeferredProofInput, PreviousOuterProofWitness,
};
use sov_rollup_interface::zk::aggregated_proof::{BlockHeaderWithProof, CodeCommitmentHash};
use sov_rollup_interface::zk::ZkvmHost;
use sp1_sdk::blocking::ProveRequest;
use sp1_sdk::blocking::{CpuProver, MockProver, Prover, ProverClient};
use sp1_sdk::ProvingKey;
use sp1_sdk::{HashableKey, SP1Proof, SP1ProvingKey, SP1Stdin};

/// SP1 host that produces aggregated (outer) proofs by recursively verifying
/// a batch of inner state-transition proofs inside an SP1 guest program.
///
/// Each call to [`run`](Self::run) generates a compressed aggregation proof
/// that covers one batch of inner proofs.  When called multiple times the host
/// automatically chains proofs: the previous aggregation proof is fed back as
/// a deferred proof input so the guest can verify continuity.
pub struct SP1AggregationHost {
    host: SP1Host<'static>,
    aggregation_vk: sp1_sdk::SP1VerifyingKey,
    code_commitment: SP1MethodId,
    inner_method_id: SP1MethodId,
    prev_agg_proof: Option<sp1_sdk::SP1ProofWithPublicValues>,
}

impl SP1AggregationHost {
    /// Creates a new aggregation host from the aggregation guest `elf` binary
    /// and the verifying key (`inner_method_id`) of the inner proof program.
    pub fn new(elf: &'static [u8], inner_method_id: SP1MethodId) -> anyhow::Result<Self> {
        let host = SP1Host::new(elf);
        let (_, pk) = host.create_prover_and_pk()?;
        let code_commitment = SP1MethodId(bincode::serialize(pk.verifying_key())?);
        Ok(Self {
            host,
            aggregation_vk: pk.verifying_key().clone(),
            code_commitment,
            inner_method_id,
            prev_agg_proof: None,
        })
    }

    /// Returns the code commitment (verifying key) of the aggregation program.
    pub fn code_commitment(&self) -> SP1MethodId {
        self.code_commitment.clone()
    }

    /// Generates a compressed aggregation proof over the supplied inner
    /// `proofs_and_headers`.  If a previous aggregation proof exists it is
    /// included as a deferred proof input for recursive verification.
    pub fn run<Da: DaSpec>(
        &mut self,
        proofs_and_headers: Vec<BlockHeaderWithProof<Da>>,
    ) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(
            !proofs_and_headers.is_empty(),
            "At least one inner proof is required"
        );

        let prev_outer_proof_witness =
            if let Some(previous_outer_proof) = self.prev_agg_proof.as_ref() {
                let serialized = bincode::serialize(previous_outer_proof)?;
                let public_values = self
                    .host
                    .add_proof_helper(&serialized, &self.code_commitment)?;

                Some(PreviousOuterProofWitness { public_values })
            } else {
                None
            };

        let mut proof_inputs = Vec::with_capacity(proofs_and_headers.len());
        for proof_and_header in proofs_and_headers {
            let public_values = self.host.add_proof_helper(
                &proof_and_header.proof.raw_inner_proof,
                &self.inner_method_id,
            )?;
            let proof_input = DeferredProofInput::<Da> {
                public_values,
                da_block_header: proof_and_header.da_block_header,
            };

            proof_inputs.push(proof_input);
        }

        let aggregation_vk_hash = self.aggregation_vk.hash_u32();
        let outer_vkey_hash = CodeCommitmentHash::from_u32_array(aggregation_vk_hash);

        let witness = AggregatedProofWitness {
            proof_inputs,
            outer_vkey_hash,
            prev_outer_proof_witness,
        };

        self.host.add_hint(witness);

        let agg_proof = self.host.run_helper()?;
        let serialized = bincode::serialize(&agg_proof)?;
        self.prev_agg_proof = Some(agg_proof);

        Ok(serialized)
    }
}

/// SP1 Host implementation.
pub struct SP1Host<'host> {
    elf: &'host [u8],
    stdin: SP1Stdin,
}

/// Instantiate a new SP1 Host.
impl<'host> SP1Host<'host> {
    /// Create a new SP1 Host.
    pub fn new(elf: &'host [u8]) -> Self {
        Self {
            elf,
            stdin: SP1Stdin::new(),
        }
    }

    /// Create a new `Sp1Guest` that reads the provided hints
    pub fn simulate_with_hints(&mut self) -> SP1Guest {
        SP1Guest::with_hints(self.stdin.buffer.clone())
    }

    fn create_prover_and_pk(&self) -> anyhow::Result<(CpuProver, SP1ProvingKey)> {
        let prover = ProverClient::builder().cpu().build();
        let pk = prover
            .setup(self.elf.into())
            .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;

        Ok((prover, pk))
    }

    fn add_proof_helper(
        &mut self,
        proof: &[u8],
        method_id: &SP1MethodId,
    ) -> anyhow::Result<Vec<u8>> {
        let proof = crate::decode_sp1_proof(proof)?;

        let SP1Proof::Compressed(recursion_proof) = &proof.proof else {
            anyhow::bail!("Expected a compressed SP1 proof");
        };
        let vk: sp1_sdk::SP1VerifyingKey = bincode::deserialize(&method_id.0)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize SP1VerifyingKey: {e}"))?;

        self.stdin
            .write_proof((**recursion_proof).clone(), vk.vk.clone());
        Ok(proof.public_values.to_vec())
    }

    fn run_helper(&mut self) -> anyhow::Result<sp1_sdk::SP1ProofWithPublicValues> {
        let stdin = std::mem::take(&mut self.stdin);

        let (prover, pk) = self.create_prover_and_pk()?;
        let output: sp1_sdk::SP1ProofWithPublicValues = prover
            .prove(&pk, stdin)
            .compressed()
            .run()
            .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;

        Ok(output)
    }
}

impl Clone for SP1Host<'_> {
    fn clone(&self) -> Self {
        Self {
            elf: self.elf,
            stdin: self.stdin.clone(),
        }
    }
}

impl core::fmt::Debug for SP1Host<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Host").finish()
    }
}

impl ZkvmHost for SP1Host<'static> {
    type HostArgs = &'static [u8];
    type Guest = SP1Guest;

    fn from_args(args: &Self::HostArgs) -> Self {
        Self::new(args)
    }

    fn add_hint<T: Serialize>(&mut self, item: T) {
        self.stdin.write(&item);
    }

    fn run(&mut self, with_proof: bool) -> anyhow::Result<Vec<u8>> {
        let output = if with_proof {
            self.run_helper()?
        } else {
            anyhow::bail!("SP1Host supports only full proofs")
        };
        Ok(bincode::serialize(&output)?)
    }

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        let (_, pk) = self.create_prover_and_pk()?;
        Ok(crate::SP1MethodId(bincode::serialize(pk.verifying_key())?))
    }
}

/// SP1 prover that uses the mock backend for fast, deterministic proving
/// without generating real cryptographic proofs.
///
/// Useful for testing and development where proof validity doesn't matter
/// but the proving pipeline needs to be exercised end-to-end.
#[derive(Clone)]
pub struct MockSp1Prover {
    elf: &'static [u8],
    stdin: SP1Stdin,
}

impl MockSp1Prover {
    /// Creates a new mock prover for the given guest ELF binary.
    pub fn new(elf: &'static [u8]) -> Self {
        Self {
            elf,
            stdin: SP1Stdin::new(),
        }
    }

    /// Writes a serializable hint value into the prover's stdin for the guest to read.
    pub fn add_hint<T: Serialize>(&mut self, item: T) {
        self.stdin.write(&item);
    }

    /// Executes the guest program and generates a compressed mock proof.
    pub fn run(&mut self) -> anyhow::Result<sp1_sdk::SP1ProofWithPublicValues> {
        let (prover, pk) = self.create_prover_and_pk()?;

        let stdin = std::mem::take(&mut self.stdin);

        let output = prover
            .prove(&pk, stdin)
            .compressed()
            .run()
            .map_err(|e| anyhow::anyhow!("SP1 proving failed. Error: {:?}", e))?;

        Ok(output)
    }

    /// Verifies a mock proof against the program's verifying key.
    pub fn verify(&self, proof: &sp1_sdk::SP1ProofWithPublicValues) -> anyhow::Result<()> {
        let (prover, pk) = self.create_prover_and_pk()?;

        prover
            .verify(proof, pk.verifying_key(), None)
            .map_err(|e| anyhow::anyhow!("SP1 verification failed. Error: {:?}", e))
    }

    fn create_prover_and_pk(&self) -> anyhow::Result<(MockProver, SP1ProvingKey)> {
        let prover = ProverClient::builder().mock().build();
        let pk = prover
            .setup(self.elf.into())
            .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;

        Ok((prover, pk))
    }
}
