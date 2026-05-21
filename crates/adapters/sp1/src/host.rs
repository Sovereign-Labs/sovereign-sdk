//! Implementation of the SP1 host for the Sovereign ZkvmHost trait.

use std::sync::{Arc, Mutex};

use crate::guest::SP1Guest;
use crate::metrics::submit_proving_metric;
use crate::SP1MethodId;
use serde::Serialize;
use sov_metrics::ZkCircuit;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::reexports::anyhow;
use sov_rollup_interface::zk::aggregated_proof::common::{
    AggregatedProofWitness, DeferredProofInput, PreviousOuterProofWitness, SerializedPubValues,
};
use sov_rollup_interface::zk::aggregated_proof::{
    BlockHeaderWithProof, BlockProof, CodeCommitmentHash, OuterZkvmHost, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{SerializedZkProof, ZkvmHost};
use sp1_sdk::blocking::{
    EnvProver, EnvProvingKey, NetworkProver, ProveRequest, Prover, ProverClient,
};
use sp1_sdk::ProvingKey;
use sp1_sdk::SP1VerifyingKey;
use sp1_sdk::{HashableKey, SP1Proof, SP1ProvingKey, SP1Stdin};

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
        Self::new_with_previous_proof(elf, inner_vk, None)
    }

    /// Like [`Self::new`], but seeded with the latest aggregated proof
    /// previously persisted in the ledger DB so that recursive verification of
    /// the previous outer proof survives a node restart.
    pub fn new_with_previous_proof(
        elf: &'static [u8],
        inner_vk: sp1_sdk::SP1VerifyingKey,
        previous_aggregated_proof: Option<SerializedAggregatedProof>,
    ) -> anyhow::Result<Self> {
        let prover = SP1Prover::new_outer(elf)?;
        let outer_vk = prover.verifying_key().clone();

        let prev_agg_proof = previous_aggregated_proof
            .map(|proof| crate::decode_sp1_proof(&proof.to_serialized_zk_proof()))
            .transpose()?;

        Ok(Self {
            inner: Arc::new(Inner {
                prover,
                outer_vk,
                inner_vk,
                prev_agg_proof: Mutex::new(prev_agg_proof),
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
    circuit: ZkCircuit,
    is_reserved_network: bool,
}

impl SP1Prover {
    /// Create a new [`SP1Prover`] for an inner state-transition circuit.
    pub fn new(elf: &[u8]) -> anyhow::Result<Self> {
        Self::with_circuit(elf, ZkCircuit::Inner)
    }

    /// Create a new [`SP1Prover`] for the outer aggregation circuit.
    pub(crate) fn new_outer(elf: &[u8]) -> anyhow::Result<Self> {
        Self::with_circuit(elf, ZkCircuit::Outer)
    }

    fn with_circuit(elf: &[u8], circuit: ZkCircuit) -> anyhow::Result<Self> {
        let (prover, pk, is_reserved_network) = prover_and_pk(elf)?;
        Ok(Self {
            prover,
            pk: Arc::new(pk),
            circuit,
            is_reserved_network,
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

    /// Sleeps for the duration (in milliseconds) read from `env_var`, but only
    /// when running under the SP1 mock backend. Used in tests/soak runs to
    /// throttle the otherwise-instant mock prover. No-op if the env var is
    /// unset or not parseable, or if the prover is not a mock.
    fn maybe_mock_sleep(&self, env_var: &str) {
        if !matches!(&self.prover, &EnvProver::Mock(_)) {
            return;
        }
        let Some(ms) = std::env::var(env_var)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return;
        };
        std::thread::sleep(std::time::Duration::from_millis(ms));
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
        // For network proving we go through the explicit two-step API
        // (`request()` + `wait_proof()`) so we can capture the request_id and
        // afterwards fetch the canonical cycles + PGUs the network recorded.
        if let EnvProver::Network(network) = &self.prover {
            let EnvProvingKey::Network { pk, .. } = &*self.pk else {
                anyhow::bail!("EnvProver is Network but proving key variant is not Network");
            };
            return self.run_network(network, pk, stdin);
        }

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

    fn run_network(
        &self,
        network: &NetworkProver,
        pk: &SP1ProvingKey,
        stdin: SP1Stdin,
    ) -> anyhow::Result<sp1_sdk::SP1ProofWithPublicValues> {
        let mut request_builder = network.prove(pk, stdin).compressed();

        if self.is_reserved_network {
            request_builder = request_builder.strategy(FulfillmentStrategy::Reserved);
        }

        let request_id = request_builder
            .request()
            .map_err(|e| anyhow::anyhow!("SP1 network proof submission failed. Error: {:?}", e))?;

        let proof = network
            .wait_proof(request_id, None, None)
            .map_err(|e| anyhow::anyhow!("SP1 network proof wait failed. Error: {:?}", e))?;

        submit_proving_metric(network, request_id, self.circuit);

        Ok(proof)
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
    let (_, pk, _) = prover_and_pk(elf)?;
    Ok(pk.verifying_key().clone())
}

/// Computes the [`SP1MethodId`] (code commitment) for the given guest ELF.
pub fn code_commitment_from_elf(elf: &[u8]) -> anyhow::Result<SP1MethodId> {
    let vk = verifying_key_from_elf(elf)?;
    Ok(code_commitment_from_verifying_key(&vk))
}

/// Computes the [`SP1MethodId`] (code commitment) for an already-derived verifying key.
pub fn code_commitment_from_verifying_key(vk: &SP1VerifyingKey) -> SP1MethodId {
    SP1MethodId(vk.hash_u32())
}

fn prover_and_pk(elf: &[u8]) -> anyhow::Result<(EnvProver, EnvProvingKey, bool)> {
    let (mut prover, is_reserved_network) = prover_client_from_env();

    let pk = prover
        .setup(elf.into())
        .map_err(|e| anyhow::anyhow!("SP1 setup failed. Error: {:?}", e))?;

    Ok((prover, pk, is_reserved_network))
}

fn prover_client_from_env() -> (EnvProver, bool) {
    let prover = match std::env::var("SP1_PROVER") {
        Ok(prover) => prover,
        Err(_) => "cpu".to_string(),
    };

    match prover.as_str() {
        "cpu" => (
            Self::Cpu(CpuProver::new_with_opts_and_machine(core_opts, machine)),
            false,
        ),
        "mock" => (Self::Mock(MockProver::new_with_machine(machine)), false),
        "light" => (Self::Light(LightProver::new_with_machine(machine)), false),

        "network" => Self::Network(Box::new((
            crate::blocking::network::builder::NetworkProverBuilder::new_with_machine(machine)
                .build(),
            false,
        ))),
        "network-reserved" => {
            let client = ProverClient::builder()
                .network_for(NetworkMode::Reserved)
                .build();

            (Self::Network(Box::new(client)), true)
        }
    }
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
        self.prover.maybe_mock_sleep("SOV_MOCK_PROVE_SLEEP_MS");
        self.add_hint_deferred_and_run_helper(item, agg_proofs)
    }

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(self.prover.method_id())
    }
}

impl OuterZkvmHost for SP1AggregationHost {
    fn run_proof_aggregation<
        Address: Serialize + Clone,
        Da: DaSpec,
        Root: Serialize + serde::de::DeserializeOwned + Clone + PartialEq + core::fmt::Debug,
    >(
        &self,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        let proofs_and_headers: Vec<BlockHeaderWithProof<Da>> = headers_with_block_proofs
            .into_iter()
            .map(|(header, proof)| BlockHeaderWithProof {
                da_block_header: header,
                proof: proof.proof,
            })
            .collect();

        self.inner
            .prover
            .maybe_mock_sleep("SOV_MOCK_AGGREGATION_SLEEP_MS");
        self.run(proofs_and_headers)
    }
}
