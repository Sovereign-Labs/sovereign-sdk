//! Utilities for benchmark transaction generation

pub mod benches;
pub mod cli;

use core::ops;
use std::io::{BufWriter, Write};

use demo_stf::runtime::{GenesisConfig, Runtime, RuntimeCall};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::prelude::arbitrary::Unstructured;
use sov_modules_api::Spec;
use sov_risc0_adapter::Risc0;
use sov_transaction_generator::generators::basic::{
    BasicCallMessageFactory, BasicChangeLogEntry, BasicModuleRef, BasicTag,
};
use sov_transaction_generator::{
    rng_utils, Distribution, GeneratedMessage, MessageValidity, State,
};

use crate::BenchSpec;

type BenchmarkModule<S> = BasicModuleRef<S, Runtime<S>>;
type BenchmarkMessageFactory<S> = BasicCallMessageFactory<S, Runtime<S>>;
type BenchmarkState<S> = State<S, BasicTag>;

/// Default buffer size used for randomization
pub const DEFAULT_RANDOMIZATION_BUFFER_SIZE: u64 = 10_000_000;

/// Maximum amount of attempts to execute arbitrary outcomes.
pub const MAX_GEN_ATTEMPTS: u64 = 10;

pub type GeneratedBatch<S> = Vec<GeneratedMessage<S, RuntimeCall<S>, BasicChangeLogEntry<S>>>;
pub type S = BenchSpec<Risc0>;
pub type RT = Runtime<S>;

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkData<S: Spec>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    Genesis(GenesisConfig<S>),
    Initialization(GeneratedBatch<S>),
    Execution {
        batches: Vec<GeneratedBatch<S>>,
        slot_number: u64,
    },
}

/// Minimal amount of information needed to deterministically reconstruct a benchmark.
#[derive(Clone)]
pub struct Benchmark<S: Spec>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// The name of the benchmark.
    pub name: String,
    /// The module distribution used
    pub module_distribution: Distribution<BenchmarkModule<S>>,
    /// Message validity distribution
    pub message_validity_distribution: Distribution<MessageValidity>,
    /// Range of transactions per batch
    pub transactions_per_batch_range: ops::RangeInclusive<u64>,
    /// Range of batches per slot
    pub batches_per_slot_range: ops::RangeInclusive<u64>,
    /// Number of slots
    pub number_of_slots: u64,
    /// The initial seed used
    pub initial_seed: u128,
    /// Randomization buffer size. Using [`DEFAULT_RANDOMIZATION_BUFFER_SIZE`] should work in practice
    pub initial_randomization_buffer_size: u64,
    /// Genesis config for the benchmark.
    pub genesis_config: GenesisConfig<S>,
}

impl<S: Spec> Benchmark<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
    S: Serialize + DeserializeOwned,
{
    /// Generates the benchmark messages for a given batch.
    pub fn try_generate_batch_messages(
        &self,
        call_message_factory: &BenchmarkMessageFactory<S>,
        state: &mut BenchmarkState<S>,
        u: &mut Unstructured,
    ) -> arbitrary::Result<Vec<GeneratedBatch<S>>> {
        // We need to clone the state in case we have to scrap it because we ran out of randomness
        let mut tmp_state = state.clone();

        let num_batches = u.int_in_range(self.batches_per_slot_range.clone())?;
        let mut slots = vec![];

        for _ in 0..num_batches {
            let num_transactions = u.int_in_range(self.transactions_per_batch_range.clone())?;
            let mut batches = vec![];

            for _ in 0..num_transactions {
                let validity = self.message_validity_distribution.select_value(u)?;

                batches.push(call_message_factory.generate_call_message(
                    &self.module_distribution,
                    u,
                    &mut tmp_state,
                    *validity,
                )?);
            }

            slots.push(batches);
        }

        // The execution has succeeded, let's replace the state
        *state = tmp_state;

        Ok(slots)
    }

    /// A helper method on the benchmark that allows to safely unwrap the return values of [`Self::try_generate_batch_messages`] that output [`arbitrary::Result`]
    fn generate_batch_messages(
        &self,
        call_message_factory: &BenchmarkMessageFactory<S>,
        state: &mut BenchmarkState<S>,
        curr_buffer_size: &mut u64,
        curr_seed: &mut u128,
    ) -> Vec<GeneratedBatch<S>> {
        for _ in 0..MAX_GEN_ATTEMPTS {
            // TODO(@theochap): we are regenerating random bytes for every slot. This is under optimal and we
            // may think of a way to optimize this and reuse randomness from the previous slot.
            let random_bytes =
                rng_utils::get_random_bytes((*curr_buffer_size).try_into().unwrap(), *curr_seed);
            *curr_seed += 1;

            let u = &mut Unstructured::new(&random_bytes[..]);

            if let Ok(messages) = self.try_generate_batch_messages(call_message_factory, state, u) {
                return messages;
            }

            // An arbitrary error was raised. Increase the buffer size and try again.
            *curr_buffer_size *= 10;
        }

        panic!("Exceeded the maximum number of generation attempts for single call messages")
    }

    /// Generate benchmark messages and dump them to a buffer. Start with a [`BenchmarkData::Initialization`]. Then write [`BenchmarkData::Execution`] messages.
    // ## TODO(@theochap):
    // We should think of storing the hashes of the benchmarks generated,
    // along with useful information for reproducibility to an human-readable file for integrity.
    pub fn generate_and_write_benchmark_messages<W: Write>(
        &self,
        writer: &mut BufWriter<W>,
    ) -> anyhow::Result<()> {
        let data = bincode::serialize(&BenchmarkData::Genesis(self.genesis_config.clone()))?;
        writer.write_all(&data)?;

        tracing::info!("Starting to generate benchmark messages");

        let mut curr_seed = self.initial_seed;
        let mut curr_buf_size = self.initial_randomization_buffer_size;

        let call_message_factory = BenchmarkMessageFactory::new();
        let state = &mut BenchmarkState::new();

        let random_bytes = rng_utils::get_random_bytes(curr_buf_size as usize, curr_seed);
        let u = &mut Unstructured::new(&random_bytes[..]);

        // We update the current seed to ensure we are not generating the same bytes again
        curr_seed += 1;

        let setup_messages = call_message_factory.generate_setup_messages(
            &self
                .module_distribution
                .inner()
                .clone()
                .into_iter()
                .map(|(_, val)| val)
                .collect::<Vec<_>>(),
            u,
            state,
        ).expect("Impossible to generate setup messages, make sure that the randomization buffer size is big enough");

        let data = bincode::serialize(&BenchmarkData::Initialization(setup_messages))?;
        writer.write_all(&data)?;

        for slot_number in 0..self.number_of_slots {
            tracing::info!(slot = slot_number, "Generating benchmark messages for slot");

            let batches = self.generate_batch_messages(
                &call_message_factory,
                state,
                &mut curr_buf_size,
                &mut curr_seed,
            );

            let data = bincode::serialize(&BenchmarkData::Execution {
                batches,
                slot_number,
            })?;

            writer.write_all(&data)?;
        }

        Ok(writer.flush()?)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Seek};
    use std::sync::Arc;

    use demo_stf::genesis_config::EvmConfig;
    use sov_address::MultiAddress;
    use sov_modules_api::Address;
    use sov_test_modules::access_pattern::AccessPatternGenesisConfig;
    use sov_test_utils::runtime::genesis::zk::config::{
        HighLevelZkGenesisConfig, MinimalZkGenesisConfig,
    };
    use sov_test_utils::runtime::sov_bank::CallMessageDiscriminants as BankDiscriminants;
    use sov_test_utils::MockZkvm;
    use sov_transaction_generator::generators::bank::BankMessageGenerator;
    use sov_transaction_generator::generators::basic::BasicBankHarness;
    use sov_transaction_generator::Percent;
    use tempfile::tempfile;

    use super::*;
    use crate::BenchSpec;

    type S = BenchSpec<MockZkvm>;

    #[test]
    fn serialize_deserialize_works() {
        let bank_harness: BenchmarkModule<S> =
            Arc::new(BasicBankHarness::new(BankMessageGenerator::new(
                Distribution::with_equiprobable_values(vec![
                    BankDiscriminants::Transfer,
                    BankDiscriminants::Burn,
                    BankDiscriminants::Mint,
                    BankDiscriminants::Freeze,
                    BankDiscriminants::CreateToken,
                ]),
                Percent::one_hundred(),
            )));

        pub const NUM_SLOTS: u64 = 5;
        pub const BATCHES_PER_SLOT: u64 = 10;
        pub const TXS_PER_BATCH: u64 = 10;

        let bench = Benchmark {
            name: "This is a test benchmark".to_string(),
            module_distribution: Distribution::with_equiprobable_values(vec![bank_harness]),
            message_validity_distribution: MessageValidity::as_distribution(Percent::fifty()),
            transactions_per_batch_range: TXS_PER_BATCH..=TXS_PER_BATCH,
            batches_per_slot_range: BATCHES_PER_SLOT..=BATCHES_PER_SLOT,
            number_of_slots: NUM_SLOTS,
            initial_seed: 0,
            initial_randomization_buffer_size: DEFAULT_RANDOMIZATION_BUFFER_SIZE,
            genesis_config: GenesisConfig::from_minimal_config(
                MinimalZkGenesisConfig::from(HighLevelZkGenesisConfig::generate_with_additional_accounts_and_code_commitments(0, Default::default(), Default::default())),
                EvmConfig::default(),
                Default::default(),
                AccessPatternGenesisConfig {
                    admin: MultiAddress::Standard(Address::from_const_slice([0; 28])),
                }
            ),
        };

        let mut temp_file = tempfile().expect("Impossible to generate a tempfile");

        bench
            .generate_and_write_benchmark_messages(&mut BufWriter::new(&mut temp_file))
            .expect("Impossible to generate benchmark messages");

        temp_file.rewind().expect("Impossible to rewind");

        let mut reader = BufReader::new(&mut temp_file);

        let genesis_config: BenchmarkData<_> =
            bincode::deserialize_from::<_, BenchmarkData<S>>(&mut reader)
                .expect("Impossible to deserialize bench data");
        assert!(matches!(genesis_config, BenchmarkData::Genesis(..)));

        let first_slot: BenchmarkData<_> =
            bincode::deserialize_from::<_, BenchmarkData<S>>(&mut reader)
                .expect("Impossible to deserialize bench data");
        assert!(matches!(first_slot, BenchmarkData::Initialization(..)));

        let mut len = 0;

        // There are only 5 slots to read from, but there is also the initialization
        while let Ok(next_slot) = bincode::deserialize_from::<_, BenchmarkData<S>>(&mut reader) {
            match next_slot {
                BenchmarkData::Execution {
                    batches,
                    slot_number,
                } => {
                    assert_eq!(slot_number, len);
                    assert_eq!(batches.len(), BATCHES_PER_SLOT as usize);
                    for batch in batches {
                        assert_eq!(batch.len(), TXS_PER_BATCH as usize);
                    }
                }
                _ => panic!("The items should be of execution type"),
            }

            len += 1;
        }

        assert_eq!(len, NUM_SLOTS, "Incorrect number of values read",);
    }
}
