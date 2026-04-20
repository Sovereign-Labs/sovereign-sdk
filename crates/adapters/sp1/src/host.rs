//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use std::sync::{Arc, Mutex};

use crate::guest::SP1Guest;
use crate::SP1MethodId;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::aggregated_proof::common::{
    AggregatedProofWitness, DeferredProofInput, PreviousOuterProofWitness,
};
use sov_rollup_interface::zk::aggregated_proof::{
    BlockHeaderWithProof, BlockProof, CodeCommitmentHash, OuterZkvmHost,
};
use sov_rollup_interface::zk::ZkvmHost;
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
    host: SP1Host,
    outer_vk: sp1_sdk::SP1VerifyingKey,
    inner_vk: sp1_sdk::SP1VerifyingKey,
    prev_agg_proof: Mutex<Option<sp1_sdk::SP1ProofWithPublicValues>>,
}

impl SP1AggregationHost {
    /// Creates a new aggregation host from the aggregation guest `elf` binary
    /// and the verifying key (`inner_method_id`) of the inner proof program.
    pub fn new(elf: &'static [u8], inner_vk: sp1_sdk::SP1VerifyingKey) -> anyhow::Result<Self> {
        let host = SP1Host::new(elf)?;
        let outer_vk = host.proving_key()?.verifying_key().clone();

        Ok(Self {
            inner: Arc::new(Inner {
                host,
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

    /// Generates a compressed aggregation proof over the supplied inner
    /// `proofs_and_headers`.  If a previous aggregation proof exists it is
    /// included as a deferred proof input for recursive verification.
    pub fn run<Da: DaSpec>(
        &self,
        proofs_and_headers: Vec<BlockHeaderWithProof<Da>>,
    ) -> anyhow::Result<Vec<u8>> {
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
            let serialized = bincode::serialize(previous_outer_proof)?;
            let public_values =
                self.inner
                    .host
                    .add_proof_helper(&mut stdin, &serialized, &self.inner.outer_vk)?;

            Some(PreviousOuterProofWitness { public_values })
        } else {
            None
        };

        let mut proof_inputs = Vec::with_capacity(proofs_and_headers.len());
        for proof_and_header in proofs_and_headers {
            let public_values = self.inner.host.add_proof_helper(
                &mut stdin,
                &proof_and_header.proof.raw_inner_proof,
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

        let agg_proof = self.inner.host.run_helper(stdin)?;
        // Downstream consumers (`sov_prover_incentives::process_proof` on
        // native, `SP1Verifier::verify` inside the STF guest) decode the same
        // `SovSP1AggregatedProof` wrapper. Native uses `serialized_sp1_proof`
        // for real verification; the guest consumes `public_values`. Emitting
        // the wrapper keeps the two sides in lock-step on the witness hint
        // stream. The full proof is also retained in `prev_agg_proof` for the
        // next aggregation round's deferred-proof witness.
        let public_values = agg_proof.public_values.to_vec();
        let serialized_sp1_proof = bincode::serialize(&agg_proof)?;
        *prev_agg_proof = Some(agg_proof);

        let wrapper = crate::SovSP1AggregatedProof {
            serialized_sp1_proof,
            public_values,
        };
        Ok(bincode::serialize(&wrapper)?)
    }
}

/// SP1 Host implementation.
#[derive(Clone)]
pub struct SP1Host {
    prover: EnvProver,
    pk: Arc<EnvProvingKey>,
}

/// Instantiate a new SP1 Host.
impl SP1Host {
    /// Create a new SP1 Host backed by a real proving key derived from `elf`.
    pub fn new(elf: &[u8]) -> anyhow::Result<Self> {
        let prover = ProverClient::from_env();

        let pk = prover
            .setup(elf.into())
            .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;

        Ok(Self {
            prover,
            pk: Arc::new(pk),
        })
    }

    /// Create a new `Sp1Guest` that reads the provided hints
    pub fn simulate_with_hints(stdin: SP1Stdin) -> SP1Guest {
        SP1Guest::with_hints(stdin.buffer)
    }

    pub(crate) fn proving_key(&self) -> anyhow::Result<&EnvProvingKey> {
        Ok(&self.pk)
    }

    /// Prepares a compressed SP1 proof for deferred verification: registers the
    /// compressed proof with `stdin` and returns the serialized
    /// [`SovSP1AggregatedProof`] wrapper that downstream guest-side
    /// `SP1Verifier::verify` expects.
    fn add_proof_helper(
        &self,
        stdin: &mut SP1Stdin,
        proof: &[u8],
        vk: &sp1_sdk::SP1VerifyingKey,
    ) -> anyhow::Result<Vec<u8>> {
        let decoded = crate::decode_sp1_proof(proof)?;

        let SP1Proof::Compressed(recursion_proof) = &decoded.proof else {
            anyhow::bail!("Expected a compressed SP1 proof");
        };

        stdin.write_proof((**recursion_proof).clone(), vk.vk.clone());

        let wrapper = crate::SovSP1AggregatedProof {
            serialized_sp1_proof: proof.to_vec(),
            public_values: decoded.public_values.to_vec(),
        };
        Ok(bincode::serialize(&wrapper)?)
    }

    fn run_helper(&self, stdin: SP1Stdin) -> anyhow::Result<sp1_sdk::SP1ProofWithPublicValues> {
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

    /// Verification key.
    pub fn verifying_key(&self) -> &SP1VerifyingKey {
        self.pk.verifying_key()
    }

    /// Method id.
    pub fn method_id(&self) -> SP1MethodId {
        SP1MethodId(self.pk.verifying_key().hash_u32())
    }
}

impl core::fmt::Debug for SP1Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sp1Host").finish()
    }
}

impl ZkvmHost for SP1Host {
    type HostArgs = &'static [u8];
    type Guest = SP1Guest;

    fn from_args(args: &Self::HostArgs) -> Self {
        Self::new(args).unwrap_or_else(|e| panic!("Failed to create SP1Host: {e:?}"))
    }

    fn add_hint_and_run<T: Serialize>(&mut self, item: &T) -> anyhow::Result<Vec<u8>> {
        let mut stdin = SP1Stdin::new();
        stdin.write(item);
        let output = self.run_helper(stdin)?;
        Ok(bincode::serialize(&output)?)
    }

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(crate::SP1MethodId(self.pk.verifying_key().hash_u32()))
    }
}

impl OuterZkvmHost for SP1AggregationHost {
    fn run_proof_aggregation<Address: Serialize + Clone, Da: DaSpec, Root: Serialize + Clone>(
        &self,
        _genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<Vec<u8>> {
        let proofs_and_headers: Vec<BlockHeaderWithProof<Da>> = headers_with_block_proofs
            .into_iter()
            .map(|(header, proof)| BlockHeaderWithProof {
                da_block_header: header,
                proof: proof.proof,
            })
            .collect();

        self.run(proofs_and_headers)
    }
}
