use std::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_macros::config_value_private;

use super::Spec;
use crate::gas::GAS_DIMENSIONS;
use crate::{new_constant, Amount, Gas};

/// The trait that defines the gas specification for the rollup.
pub trait GasSpec:
    BorshDeserialize + BorshSerialize + Default + Debug + Clone + Send + Sync + PartialEq + 'static
{
    /// The type of gas used in the rollup.
    type Gas: Gas;

    /// The fixed gas price of checking forced sequencer registration transactions.
    /// This price is added to regular transaction checks & execution costs.
    /// This should be set in such a way that forced sequencer registration is more expensive
    /// than regular registration to prevent this mechanism being gamed instead of
    /// used only when users feel they are being censored.
    fn gas_forced_sequencer_registration_cost() -> Self::Gas;

    // --- Gas parameters to charge for state accesses ---

    /// Returns the gas to charge for accessing a value from the storage.
    fn bias_to_charge_for_access() -> Self::Gas;

    /// The cost of encoding and storing storage bytes to cache.
    fn gas_to_charge_per_byte_storage_update() -> Self::Gas;

    /// The cost of encoding and storing storage bytes to cache.
    fn bias_to_charge_storage_update() -> Self::Gas;

    /// The cost of reading a storage value from the cache.
    fn bias_to_charge_for_read() -> Self::Gas;

    /// The cost of reading a storage value into the cache.
    fn gas_to_charge_per_byte_read() -> Self::Gas;

    // --- End Gas parameters to charge for state accesses ---

    // --- Gas parameters to specify how to charge gas for hashing ---
    /// The cost per byte of updating a hash.
    fn gas_to_charge_per_byte_hash_update() -> Self::Gas;
    /// The base cost of updating a hasher.
    fn gas_to_charge_hash_update() -> Self::Gas;

    // --- End Gas parameters to specify how to charge gas for hashing ---

    // --- Gas parameters to specify how to charge gas for signature verification ---
    /// The cost of verifying a signature per byte of the signature
    fn gas_to_charge_per_byte_signature_verification() -> Self::Gas;
    /// The fixed cost of verifying a signature
    fn fixed_gas_to_charge_per_signature_verification() -> Self::Gas;
    // --- End Gas parameters to specify how to charge gas for signature verification ---

    /// The cost of deserializing a message using Borsh
    fn gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas;
    /// The bias to charge for deserializing a message using Borsh
    fn bias_borsh_deserialization() -> Self::Gas;

    /// The cost of deserializing a transaction using Borsh
    fn tx_gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas;
    /// The bias to charge for deserializing a tx using Borsh
    fn tx_bias_borsh_deserialization() -> Self::Gas;

    /// The cost of deserializing a proof using Borsh
    fn proof_gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas;
    /// The bias to charge for deserializing a proof using Borsh
    fn proof_bias_borsh_deserialization() -> Self::Gas;

    /// The cost of deserializing a sample string using Borsh
    fn string_gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas;
    /// The bias to charge for deserializing a sample string using Borsh
    fn string_bias_borsh_deserialization() -> Self::Gas;

    // --- Gas fee adjustment parameters: See https://eips.ethereum.org/EIPS/eip-1559 for a detailed description ---
    /// The initial gas limit of the rollup.
    fn initial_gas_limit() -> Self::Gas;
    /// The initial "base fee" that every transaction emits when executed.
    fn initial_base_fee_per_gas() -> <Self::Gas as Gas>::Price;

    /// Maximum amount of gas the sequencer can pay for the tx execution. Typically this will be the sum
    /// of authentication (sig check) gas and `process_tx_pre_exec_checks_gas()`.
    fn max_tx_check_costs() -> Self::Gas;

    /// Maximum amount of gas that can be charged for sequencer registration.
    fn max_unregistered_tx_check_costs() -> Self::Gas;

    /// The gas used for the transaction pre-execution checks.
    /// For example nonce checks, context resolution etc..
    fn process_tx_pre_exec_checks_gas() -> Self::Gas;

    /// The cost of `CredentialId` calculation
    fn gas_to_charge_for_credential() -> Self::Gas;
    /// The gas used for the transaction pre-execution checks.
    /// For example nonce checks, context resolution etc..
    /// Charged per transaction byte.
    fn process_tx_pre_exec_checks_gas_per_tx_byte() -> Self::Gas;

    // --- Gas parameters to specify how to charge gas for zk-proof verification ---
    /// Gas parameter for zk-proof verification. Charged per proof byte.
    fn gas_to_charge_per_proof_byte() -> Self::Gas;

    /// Gas parameter for zk-proof verification
    fn fixed_gas_to_charge_per_proof() -> Self::Gas;
}

impl<S: Spec> GasSpec for S {
    type Gas = S::Gas;

    fn gas_to_charge_per_proof_byte() -> Self::Gas {
        new_constant!("GAS_TO_CHARGE_PER_PROOF_BYTE", Self::Gas)
    }

    fn fixed_gas_to_charge_per_proof() -> Self::Gas {
        new_constant!("FIXED_GAS_TO_CHARGE_PER_PROOF", Self::Gas)
    }

    // -- Begin of gas costs for accessing storage

    fn bias_to_charge_for_access() -> Self::Gas {
        new_constant!("GAS_TO_CHARGE_PER_STORAGE_ACCESS", Self::Gas)
    }

    fn bias_to_charge_for_read() -> Self::Gas {
        new_constant!("GAS_TO_CHARGE_PER_READ", Self::Gas)
    }

    fn gas_to_charge_per_byte_read() -> Self::Gas {
        new_constant!("GAS_TO_CHARGE_PER_BYTE_READ", Self::Gas)
    }

    fn bias_to_charge_storage_update() -> Self::Gas {
        new_constant!("BIAS_STORAGE_UPDATE", Self::Gas)
    }

    fn gas_to_charge_per_byte_storage_update() -> Self::Gas {
        new_constant!("GAS_TO_CHARGE_PER_BYTE_STORAGE_UPDATE", Self::Gas)
    }

    // -- End of gas costs for accessing storage

    fn gas_forced_sequencer_registration_cost() -> Self::Gas {
        new_constant!("GAS_FORCED_SEQUENCER_REGISTRATION_COST", Self::Gas)
    }

    fn fixed_gas_to_charge_per_signature_verification() -> Self::Gas {
        new_constant!(
            "DEFAULT_FIXED_GAS_TO_CHARGE_PER_SIGNATURE_VERIFICATION",
            Self::Gas
        )
    }

    fn gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas {
        new_constant!(
            "DEFAULT_GAS_TO_CHARGE_PER_BYTE_BORSH_DESERIALIZATION",
            Self::Gas
        )
    }

    fn bias_borsh_deserialization() -> Self::Gas {
        new_constant!("BIAS_BORSH_DESERIALIZATION", Self::Gas)
    }

    fn tx_gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas {
        new_constant!("TX_GAS_TO_CHARGE_PER_BYTE_BORSH_DESERIALIZATION", Self::Gas)
    }

    fn tx_bias_borsh_deserialization() -> Self::Gas {
        new_constant!("TX_BIAS_BORSH_DESERIALIZATION", Self::Gas)
    }

    fn proof_gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas {
        new_constant!(
            "PROOF_GAS_TO_CHARGE_PER_BYTE_BORSH_DESERIALIZATION",
            Self::Gas
        )
    }

    fn proof_bias_borsh_deserialization() -> Self::Gas {
        new_constant!("PROOF_BIAS_BORSH_DESERIALIZATION", Self::Gas)
    }

    fn string_gas_to_charge_per_byte_borsh_deserialization() -> Self::Gas {
        new_constant!(
            "STRING_GAS_TO_CHARGE_PER_BYTE_BORSH_DESERIALIZATION",
            Self::Gas
        )
    }

    fn string_bias_borsh_deserialization() -> Self::Gas {
        new_constant!("STRING_BIAS_BORSH_DESERIALIZATION", Self::Gas)
    }

    fn gas_to_charge_hash_update() -> Self::Gas {
        new_constant!("GAS_TO_CHARGE_HASH_UPDATE", Self::Gas)
    }

    fn gas_to_charge_per_byte_hash_update() -> Self::Gas {
        new_constant!("GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE", Self::Gas)
    }

    fn gas_to_charge_per_byte_signature_verification() -> Self::Gas {
        new_constant!(
            "DEFAULT_GAS_TO_CHARGE_PER_BYTE_SIGNATURE_VERIFICATION",
            Self::Gas
        )
    }

    fn initial_base_fee_per_gas() -> <Self::Gas as Gas>::Price {
        let raw: [u128; GAS_DIMENSIONS] = config_value_private!("INITIAL_BASE_FEE_PER_GAS");
        let actual: [Amount; GAS_DIMENSIONS] = raw.map(Amount::from);
        <Self::Gas as Gas>::Price::from(actual)
    }

    fn initial_gas_limit() -> Self::Gas {
        Self::Gas::from(config_value_private!("INITIAL_GAS_LIMIT"))
    }

    fn max_tx_check_costs() -> Self::Gas {
        new_constant!("MAX_SEQUENCER_EXEC_GAS_PER_TX", Self::Gas)
    }

    fn max_unregistered_tx_check_costs() -> Self::Gas {
        new_constant!("MAX_UNREGISTERED_SEQUENCER_EXEC_GAS_PER_TX", Self::Gas)
    }

    fn process_tx_pre_exec_checks_gas() -> Self::Gas {
        new_constant!("PROCESS_TX_PRE_EXEC_GAS", Self::Gas)
    }

    fn gas_to_charge_for_credential() -> Self::Gas {
        Self::Gas::from(config_value_private!(
            "GAS_TO_CHARGE_FOR_CREDENTIAL_CALCULATION"
        ))
    }

    fn process_tx_pre_exec_checks_gas_per_tx_byte() -> Self::Gas {
        new_constant!("PROCESS_TX_PRE_EXEC_GAS_PER_TX_BYTE", Self::Gas)
    }
}
