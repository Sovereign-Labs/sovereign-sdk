/// A module for testing gas charges
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{
    Context, DaSpec, GenesisState, HexHash, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec,
    StateMap, TxState,
};
use sov_state::pinned_cache::BucketId;

/// A message to test and set a value
#[derive(
    Clone,
    BorshSerialize,
    BorshDeserialize,
    PartialEq,
    Eq,
    Debug,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub enum CallMessage {
    /// Tests and set a value, then conditionally undoes the changes without reverting.
    TestCacheAccesses {
        address: HexHash,
        read_indexes: Option<ValueRange>,
        write_indexes: Option<ValueRange>,
        expected_storage_accesses: Option<u64>,
    },
}

#[derive(
    Clone,
    BorshSerialize,
    BorshDeserialize,
    Debug,
    Hash,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub struct ValueRange {
    pub indices: std::ops::Range<u32>,
    pub value: u32,
}

/// A module for testing the block-level cache.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct PinnedCacheTester<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    #[state]
    pub values: StateMap<StateKey, u32>,

    #[phantom]
    _phantom: std::marker::PhantomData<S>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Hash, BorshDeserialize, BorshSerialize)]
pub struct StateKey {
    address: HexHash,
    index: u32,
}

impl std::fmt::Display for StateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.address, self.index)
    }
}
impl std::str::FromStr for StateKey {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (address, index) = s
            .split_once("/")
            .ok_or(anyhow::anyhow!("Invalid state key"))?;
        Ok(StateKey {
            address: HexHash::from_str(address)?,
            index: index.parse()?,
        })
    }
}

impl<S: Spec> PinnedCacheTester<S> {
    /// Get the bucket ID for a given address.
    pub fn get_bucket_id(&self, address: &HexHash) -> BucketId {
        let key = StateKey {
            address: *address,
            index: 0,
        };
        BucketId::from_slot_key(&self.values.slot_key(&key), 32)
    }
}

impl<S: Spec> Module for PinnedCacheTester<S> {
    type Error = anyhow::Error;

    type Spec = S;

    type Config = ();

    type CallMessage = CallMessage;

    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let accesses_before = state.metrics().total_read_misses;
        match msg {
            CallMessage::TestCacheAccesses {
                address,
                read_indexes,
                write_indexes,
                expected_storage_accesses,
            } => {
                if let Some(read_indexes) = read_indexes {
                    for index in read_indexes.indices {
                        let key = StateKey { index, address };
                        let value = self.values.get(&key, state)?;
                        assert_eq!(
                            value.unwrap_or_default(),
                            read_indexes.value,
                            "Expected value {} for index {}, but got {}",
                            read_indexes.value,
                            index,
                            value.unwrap_or_default()
                        );
                    }
                }
                if let Some(write_indexes) = write_indexes {
                    for index in write_indexes.indices {
                        let key = StateKey { index, address };
                        self.values.set(&key, &write_indexes.value, state)?;
                    }
                }

                if let Some(expected_storage_accesses) = expected_storage_accesses {
                    let num_accesses = state.metrics().total_read_misses - accesses_before;
                    assert_eq!(
                        num_accesses, expected_storage_accesses,
                        "Unexpected number of storage accesses. Expected: {expected_storage_accesses}, Actual: {num_accesses}",
                    );
                }
                Ok(())
            }
        }
    }
}
