use std::fmt::Debug;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use schemars::JsonSchema;
use sov_modules_api::digest::Digest;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CryptoSpec, EventEmitter, Spec, TxState};
use strum::{EnumDiscriminants, EnumIs, VariantArray};

use super::{SyntheticLoad, VERY_LARGE_VEC_LENGTH};
use crate::event::Event;

/// This enumeration represents the available call messages for interacting with the module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, EnumDiscriminants, EnumIs, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(rename = "CallMessage")]
#[strum_discriminants(derive(VariantArray, EnumIs))]
#[serde(rename_all = "snake_case")]
pub enum CallMessage {
    /// Read and set many individual values.
    ReadAndSetManyIndividualValues {
        /// The number of values to read and set.
        number_of_operations: u64,
        /// The salt.
        salt: u64,
    },
    /// Read and set entries in a large vector stored as a `StateValue`
    ReadAndSetHeavyState {
        /// The number of new values to read and set.
        number_of_new_values: u64,
        /// The max size of the heavy state.
        max_heavy_state_size: u64,
        /// The salt.
        salt: u64,
    },
    /// Run CPU heavy operation. Each iteration computes a hash with the Spec::Hasher.
    RunCPUHeavyOperation {
        /// The number of iterations.
        iterations: u64,
    },
}

impl<S: Spec> SyntheticLoad<S> {
    pub(crate) fn read_and_set_many_individual_values(
        &mut self,
        number_of_operations: u64,
        salt: u64,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        // Use a ChaCha8-based PRNG seeded with the supplied salt.
        let mut rng = ChaCha8Rng::seed_from_u64(salt);

        for _ in 0..number_of_operations {
            // Generate pseudo-random indices for read and write operations.
            let read_index: u64 = rng.gen_range(0..VERY_LARGE_VEC_LENGTH);
            let write_index: u64 = rng.gen_range(0..VERY_LARGE_VEC_LENGTH);

            // Read from a random location.
            let value_read = self.very_large_vec.get(read_index, state)?.unwrap_or(0);
            let value_to_write = value_read.wrapping_add(1);

            // Write to the random location
            let _ = self
                .very_large_vec
                .set(write_index, &value_to_write, state)?;
        }

        self.emit_event(
            state,
            Event::ReadAndSetManyIndividualValues(number_of_operations),
        );
        Ok(())
    }

    pub(crate) fn read_and_set_heavy_state(
        &mut self,
        number_of_new_values: u64,
        max_heavy_state_size: u64,
        salt: u64,
        _context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        // Read the entire huge vector from the state.
        let mut old_vec: Vec<u64> = self.heavy_state.get(state)?.unwrap_or_default();

        // Modify the vector by appending new values.
        let new_values = vec![salt; number_of_new_values as usize];
        old_vec.extend(new_values);

        // Truncate if the vector is over the size limit.
        // This removes the oldest data (from the front) to make room for the new data.
        if old_vec.len() > max_heavy_state_size as usize {
            let num_to_remove = old_vec.len() - max_heavy_state_size as usize;
            old_vec.drain(0..num_to_remove);
        }

        // Write the entire modified vector back to state.
        self.heavy_state.set::<Vec<u64>, _>(&old_vec, state)?;

        self.emit_event(
            state,
            Event::ReadAndSetHeavyState(number_of_new_values, salt),
        );

        Ok(())
    }

    pub(crate) fn run_cpu_heavy_operation(
        &mut self,
        iterations: u64,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if iterations == u64::MAX {
            anyhow::bail!("u64 is too many operations, even for heavy load");
        }
        let mut current_hash =
            <S::CryptoSpec as CryptoSpec>::Hasher::digest(context.sender().as_ref());
        for _ in 0..iterations {
            current_hash = <S::CryptoSpec as CryptoSpec>::Hasher::digest(&current_hash[..]);
        }
        self.emit_event(
            state,
            Event::RanCPUHeavyOperation(iterations, current_hash.to_vec()),
        );
        Ok(())
    }
}
