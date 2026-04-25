use std::sync::{Arc, Mutex};

use crate::notifier::NotificationManager;
use crate::{MockCodeCommitment, MockProof, MockZkGuest, MockZkVerifier};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, BlockProof, OuterZkvmHost, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::SerializedZkProof;

/// A mock implementing the zkVM trait.
#[derive(Clone)]
pub struct MockZkvmHost {
    notification_manager: NotificationManager,
    prev_aggregated_proof: Arc<Mutex<Option<SerializedAggregatedProof>>>,
    wait_for_proof: bool,
}

impl MockZkvmHost {
    /// Creates a new MockZkvm.
    pub fn new() -> Self {
        Self {
            wait_for_proof: true,
            notification_manager: Default::default(),
            prev_aggregated_proof: Default::default(),
        }
    }

    /// Creates a new MockZkvm, the `ZkvmHost::add_hint_and_run` will return immediately.
    pub fn new_non_blocking() -> Self {
        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
            prev_aggregated_proof: Default::default(),
        }
    }

    /// Simulates zk proof generation.
    pub fn make_proof(&self) {
        // We notify the worker thread.
        self.notification_manager.notify();
    }

    /// Create a proof for MockZkvm
    pub fn create_serialized_proof<T: Serialize>(
        is_valid: bool,
        transition: T,
    ) -> SerializedZkProof {
        let data = bincode::serialize(&transition).unwrap();
        let raw_proof = bincode::serialize(&MockProof {
            is_valid,
            pub_data: data,
        })
        .unwrap();
        SerializedZkProof { raw_proof }
    }

    fn add_hint_and_run_inner<T: Serialize>(&self, item: &T) -> anyhow::Result<Vec<u8>> {
        let pub_data = bincode::serialize(item)?;
        if self.wait_for_proof {
            self.notification_manager.wait();
        }
        Ok(bincode::serialize(&MockProof {
            is_valid: true,
            pub_data,
        })?)
    }

    fn deserialize_aggregated_proof<
        Address: DeserializeOwned,
        Da: DaSpec,
        Root: DeserializeOwned,
    >(
        aggregated_proof: &SerializedAggregatedProof,
    ) -> anyhow::Result<AggregatedProofPublicData<Address, Da, Root>> {
        <MockZkVerifier as sov_rollup_interface::zk::ZkVerifier>::verify_with_proof(
            &SerializedZkProof {
                raw_proof: aggregated_proof.raw_aggregated_proof.clone(),
            },
            &MockCodeCommitment::default(),
        )
    }

    fn ensure_same_serialized_value<T: Serialize>(
        left: &T,
        right: &T,
        message: &'static str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            bincode::serialize(left)? == bincode::serialize(right)?,
            message
        );
        Ok(())
    }

    fn assert_batch_continuity<Address: Serialize + Clone, Da: DaSpec, Root: Serialize + Clone>(
        block_proofs_data: &[&BlockProof<Address, Da, Root>],
    ) -> anyhow::Result<()> {
        for block_proofs in block_proofs_data.windows(2) {
            let [prev, next] = block_proofs else {
                continue;
            };

            let expected_next_slot = prev.slot_number.saturating_add(1);
            anyhow::ensure!(
                next.slot_number == expected_next_slot,
                "Mock outer proof continuity broken: expected next block proof at slot {expected_next_slot}, got {}",
                next.slot_number,
            );
            Self::ensure_same_serialized_value(
                &prev.st.final_state_root,
                &next.st.initial_state_root,
                "Mock outer proof continuity broken: adjacent block proofs do not connect",
            )?;
        }

        Ok(())
    }

    fn assert_aggregated_proof_continuity<
        Address: Serialize + DeserializeOwned + Clone,
        Da: DaSpec,
        Root: Serialize + DeserializeOwned + Clone,
    >(
        prev_aggregated_proof: &SerializedAggregatedProof,
        public_data: &AggregatedProofPublicData<Address, Da, Root>,
    ) -> anyhow::Result<()> {
        let prev_public_data =
            Self::deserialize_aggregated_proof::<Address, Da, Root>(prev_aggregated_proof)?;
        let expected_next_slot = prev_public_data.final_slot_number.saturating_add(1);

        anyhow::ensure!(
            public_data.initial_slot_number == expected_next_slot,
            "Mock outer proof continuity broken: expected aggregated proof to start at slot {expected_next_slot}, got {}",
            public_data.initial_slot_number,
        );
        Self::ensure_same_serialized_value(
            &prev_public_data.final_state_root,
            &public_data.initial_state_root,
            "Mock outer proof continuity broken: previous final state root does not match next initial state root",
        )?;
        Self::ensure_same_serialized_value(
            &prev_public_data.genesis_state_root,
            &public_data.genesis_state_root,
            "Mock outer proof continuity broken: genesis state root changed between aggregated proofs",
        )?;

        Ok(())
    }
}

impl Default for MockZkvmHost {
    fn default() -> Self {
        Self::new()
    }
}

impl sov_rollup_interface::zk::ZkvmHost for MockZkvmHost {
    type Guest = MockZkGuest;

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(MockCodeCommitment::default())
    }

    fn add_hint_deferred_and_run<T: Serialize>(
        &mut self,
        item: &T,
        _agg_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<SerializedZkProof> {
        self.add_hint_and_run_inner(item)
            .map(|raw_proof| SerializedZkProof { raw_proof })
    }
}

impl OuterZkvmHost for MockZkvmHost {
    fn run_proof_aggregation<
        Address: Serialize + DeserializeOwned + Clone,
        Da: DaSpec,
        Root: Serialize + DeserializeOwned + Clone,
    >(
        &self,
        genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        let block_proofs_data = headers_with_block_proofs
            .iter()
            .map(|(_, bp)| bp)
            .collect::<Vec<_>>();

        let prev_aggregated_proof = self
            .prev_aggregated_proof
            .lock()
            .map_err(|e| anyhow::anyhow!("prev_aggregated_proof mutex poisoned: {e}"))?
            .clone();

        Self::assert_batch_continuity(block_proofs_data.as_slice())?;

        let public_data = AggregatedProofPublicData::from_block_proofs(
            block_proofs_data.as_slice(),
            genesis_state_root,
        );
        if let Some(prev_aggregated_proof) = prev_aggregated_proof.as_ref() {
            Self::assert_aggregated_proof_continuity::<Address, Da, Root>(
                prev_aggregated_proof,
                &public_data,
            )?;
        }

        let aggregated_proof =
            self.add_hint_and_run_inner(&public_data)
                .map(|raw_aggregated_proof| SerializedAggregatedProof {
                    raw_aggregated_proof,
                })?;

        *self
            .prev_aggregated_proof
            .lock()
            .map_err(|e| anyhow::anyhow!("prev_aggregated_proof mutex poisoned: {e}"))? =
            Some(aggregated_proof.clone());

        Ok(aggregated_proof)
    }

    fn restore_persisted_aggregated_proof(
        &self,
        aggregated_proof: SerializedAggregatedProof,
    ) -> anyhow::Result<()> {
        *self
            .prev_aggregated_proof
            .lock()
            .map_err(|e| anyhow::anyhow!("prev_aggregated_proof mutex poisoned: {e}"))? =
            Some(aggregated_proof);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::{Display, Formatter};
    use std::str::FromStr;

    use borsh::{BorshDeserialize, BorshSerialize};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use sov_rollup_interface::common::SlotNumber;
    use sov_rollup_interface::da::{
        BlobReaderTrait, BlockHashTrait, BlockHeaderTrait, DaSpec, Time,
    };
    use sov_rollup_interface::zk::StateTransitionPublicData;
    use sov_rollup_interface::BasicAddress;
    use sov_universal_wallet::UniversalWallet;

    use super::*;

    #[derive(
        Clone,
        Copy,
        Debug,
        Default,
        Eq,
        PartialEq,
        Hash,
        Serialize,
        Deserialize,
        BorshSerialize,
        BorshDeserialize,
        JsonSchema,
        UniversalWallet,
    )]
    struct TestHash([u8; 32]);

    impl AsRef<[u8]> for TestHash {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }

    impl From<TestHash> for [u8; 32] {
        fn from(value: TestHash) -> Self {
            value.0
        }
    }

    impl TryFrom<[u8; 32]> for TestHash {
        type Error = core::convert::Infallible;

        fn try_from(value: [u8; 32]) -> Result<Self, Self::Error> {
            Ok(Self(value))
        }
    }

    impl Display for TestHash {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "0x{}", hex::encode(self.0))
        }
    }

    impl FromStr for TestHash {
        type Err = hex::FromHexError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut array = [0u8; 32];
            hex::decode_to_slice(s.trim_start_matches("0x"), &mut array)?;
            Ok(Self(array))
        }
    }

    impl BlockHashTrait for TestHash {}

    #[derive(
        Clone,
        Copy,
        Debug,
        Default,
        Eq,
        PartialEq,
        Ord,
        PartialOrd,
        Hash,
        Serialize,
        Deserialize,
        BorshSerialize,
        BorshDeserialize,
        JsonSchema,
        UniversalWallet,
    )]
    struct TestDaAddress([u8; 32]);

    impl AsRef<[u8]> for TestDaAddress {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }

    impl Display for TestDaAddress {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "0x{}", hex::encode(self.0))
        }
    }

    impl FromStr for TestDaAddress {
        type Err = hex::FromHexError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut array = [0u8; 32];
            hex::decode_to_slice(s.trim_start_matches("0x"), &mut array)?;
            Ok(Self(array))
        }
    }

    impl TryFrom<&[u8]> for TestDaAddress {
        type Error = anyhow::Error;

        fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
            let array: [u8; 32] = value
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected 32-byte address"))?;
            Ok(Self(array))
        }
    }

    impl BasicAddress for TestDaAddress {}

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    struct TestBlobTx {
        sender: TestDaAddress,
        hash: TestHash,
        data: Vec<u8>,
        verified_len: usize,
    }

    impl BlobReaderTrait for TestBlobTx {
        type Address = TestDaAddress;
        type BlobHash = TestHash;

        fn sender(&self) -> Self::Address {
            self.sender
        }

        fn hash(&self) -> Self::BlobHash {
            self.hash
        }

        fn verified_data(&self) -> &[u8] {
            &self.data[..self.verified_len]
        }

        fn total_len(&self) -> usize {
            self.data.len()
        }

        fn advance(&mut self, num_bytes: usize) -> &[u8] {
            self.verified_len = self
                .verified_len
                .saturating_add(num_bytes)
                .min(self.data.len());
            &self.data[..self.verified_len]
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    struct TestHeader {
        prev_hash: TestHash,
        hash: TestHash,
        height: u64,
        time: Time,
    }

    impl BlockHeaderTrait for TestHeader {
        type Hash = TestHash;

        fn prev_hash(&self) -> Self::Hash {
            self.prev_hash
        }

        fn hash(&self) -> Self::Hash {
            self.hash
        }

        fn height(&self) -> u64 {
            self.height
        }

        fn time(&self) -> Time {
            self.time.clone()
        }
    }

    #[derive(
        Clone,
        Debug,
        Default,
        PartialEq,
        Eq,
        Serialize,
        Deserialize,
        BorshSerialize,
        BorshDeserialize,
    )]
    struct TestDaSpec;

    impl DaSpec for TestDaSpec {
        type SlotHash = TestHash;
        type BlockHeader = TestHeader;
        type BlobTransaction = TestBlobTx;
        type TransactionId = u64;
        type Address = TestDaAddress;
        type InclusionMultiProof = ();
        type CompletenessProof = ();
        type ChainParams = ();
    }

    fn make_block_proof(
        slot_number: u64,
        initial_state_root: Vec<u8>,
        final_state_root: Vec<u8>,
    ) -> (TestHeader, BlockProof<u64, TestDaSpec, Vec<u8>>) {
        let hash = TestHash([slot_number as u8; 32]);
        let prev_hash = TestHash([slot_number.saturating_sub(1) as u8; 32]);
        let header = TestHeader {
            prev_hash,
            hash,
            height: slot_number,
            time: Time::from_secs(slot_number as i64),
        };
        let block_proof = BlockProof {
            proof: MockZkvmHost::create_serialized_proof(true, slot_number),
            slot_number: SlotNumber::new_dangerous(slot_number),
            st: StateTransitionPublicData {
                initial_state_root,
                final_state_root,
                slot_hash: hash,
                prover_address: slot_number,
            },
        };

        (header, block_proof)
    }

    #[test]
    fn test_run_proof_aggregation_tracks_previous_proof() {
        let host = MockZkvmHost::new_non_blocking();
        let genesis_state_root = vec![1u8, 2, 3];

        let first_proof = host
            .run_proof_aggregation(
                genesis_state_root.clone(),
                vec![
                    make_block_proof(0, vec![10], vec![11]),
                    make_block_proof(1, vec![11], vec![12]),
                ],
            )
            .unwrap();

        let second_proof = host
            .run_proof_aggregation(
                genesis_state_root,
                vec![make_block_proof(2, vec![12], vec![13])],
            )
            .unwrap();

        let first_public_data =
            MockZkvmHost::deserialize_aggregated_proof::<u64, TestDaSpec, Vec<u8>>(&first_proof)
                .unwrap();
        let second_public_data =
            MockZkvmHost::deserialize_aggregated_proof::<u64, TestDaSpec, Vec<u8>>(&second_proof)
                .unwrap();

        assert_eq!(
            first_public_data.final_slot_number.saturating_add(1),
            second_public_data.initial_slot_number
        );
    }

    #[test]
    fn test_restore_persisted_aggregated_proof_rejects_gap() {
        let host = MockZkvmHost::new_non_blocking();
        let genesis_state_root = vec![1u8, 2, 3];

        let first_proof = host
            .run_proof_aggregation(
                genesis_state_root.clone(),
                vec![make_block_proof(0, vec![10], vec![11])],
            )
            .unwrap();

        let restored_host = MockZkvmHost::new_non_blocking();
        restored_host
            .restore_persisted_aggregated_proof(first_proof)
            .unwrap();

        let err = restored_host
            .run_proof_aggregation(
                genesis_state_root,
                vec![make_block_proof(2, vec![11], vec![12])],
            )
            .unwrap_err();

        assert!(err
            .to_string()
            .contains("expected aggregated proof to start at slot 1"));
    }

    #[test]
    fn test_run_proof_aggregation_rejects_discontinuous_block_proofs() {
        let host = MockZkvmHost::new_non_blocking();

        let err = host
            .run_proof_aggregation(
                vec![1u8, 2, 3],
                vec![
                    make_block_proof(0, vec![10], vec![11]),
                    make_block_proof(2, vec![11], vec![12]),
                ],
            )
            .unwrap_err();

        assert!(err
            .to_string()
            .contains("expected next block proof at slot 1"));
    }
}
