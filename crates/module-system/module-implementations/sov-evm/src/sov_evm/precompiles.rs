//! Stateful precompile support for sovereign EVM.
//!
//! This module provides the infrastructure for custom precompiles that can access
//! sovereign SDK state during EVM execution. Precompile implementations are defined
//! statically in code, while activation is controlled via on-chain state.
//!
//! # Architecture
//!
//! - `SovPrecompiles`: `PrecompileProvider` implementation for revm integration
//! - `invoke_precompile`: Method that dispatches to precompile implementations with state access
//! - `is_known_sov_precompile`: Function to check if an address has a precompile implementation
//!
//! # Activation Flow
//!
//! 1. Precompile code is added via software update (in `invoke_precompile` match)
//! 2. Admin enables the precompile address via transaction
//! 3. Precompile becomes callable from EVM contracts
//!
//! # Adding New Precompiles
//!
//! To add a new stateful precompile:
//! 1. Add the address constant (e.g., `BRIDGE_PRECOMPILE_ADDRESS`)
//! 2. Add a match arm in `is_known_sov_precompile` for the address
//! 3. Add a match arm in `invoke_precompile` that calls your implementation
//! 4. Implement the precompile logic (can access TxState)

use alloy_primitives::{Address, Bytes, U256};
use revm::context_interface::ContextTr;
use revm::handler::{EthPrecompiles, PrecompileProvider};
use revm::interpreter::{CallInputs, Gas, InstructionResult, InterpreterResult};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_bank::{config_gas_token_id, TokenId};
use sov_modules_api::{Spec, TxState};

/// Trait for databases that support stateful precompile execution.
///
/// This trait allows precompiles to access sovereign SDK state through the
/// EVM context's database. Implemented by `EvmDb`.
pub trait PrecompileDb<S: Spec> {
    /// The state type that provides access to sovereign SDK state.
    type State: TxState<S>;

    /// Get mutable access to the underlying state for precompile execution.
    fn precompile_state_mut(&mut self) -> &mut Self::State;
}

/// Blanket implementation for mutable references to databases that implement PrecompileDb.
impl<S: Spec, T: PrecompileDb<S>> PrecompileDb<S> for &mut T {
    type State = T::State;

    fn precompile_state_mut(&mut self) -> &mut Self::State {
        (*self).precompile_state_mut()
    }
}

/// Result type for stateful precompile execution.
pub type PrecompileResult = Result<PrecompileOutput, PrecompileError>;

/// Output from a successful precompile execution.
#[derive(Debug, Clone)]
pub struct PrecompileOutput {
    /// Gas consumed by the precompile.
    pub gas_used: u64,
    /// Output bytes returned by the precompile.
    pub bytes: Bytes,
}

/// Error from a failed precompile execution.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PrecompileError {
    /// Precompile ran out of gas.
    OutOfGas,
    /// Generic error with message.
    Error(String),
}

// =============================================================================
// Precompile Address Constants
// =============================================================================

/// Identity precompile address (0x010000).
///
/// This is a test precompile that returns its input unchanged.
/// Gas cost: 15 base + 3 per word (matching EIP-198 ecrecover style).
pub const IDENTITY_PRECOMPILE_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00, 0x00,
]);

/// Bank balance precompile address (0x010000).
///
/// Returns the bank balance for a given address and token.
/// Input: 20-byte address + optional 32-byte token ID (defaults to gas token)
/// Output: 32-byte U256 balance
/// Gas cost: 100 (fixed cost for state read)
pub const BANK_BALANCE_PRECOMPILE_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00, 0x01,
]);

/// Check if an address has a known precompile implementation in code.
///
/// Precompiles are only callable if they are both:
/// 1. Known (this function returns true)
/// 2. Enabled in the `enabled_custom_precompiles` state (activation via admin tx)
///
/// When adding a new precompile, add a match arm here for its address.
pub fn is_known_sov_precompile(address: &Address) -> bool {
    matches!(
        *address,
        IDENTITY_PRECOMPILE_ADDRESS | BANK_BALANCE_PRECOMPILE_ADDRESS
    )
}

/// PrecompileProvider that supports both standard Ethereum precompiles
/// and custom stateful sovereign precompiles.
///
/// This struct is created fresh for each transaction execution and holds
/// references to the enabled precompile addresses and bank module.
/// Stateful precompiles are invoked via the `run` method which
/// accesses `TxState` through the context's database.
pub struct SovPrecompiles<'a, S: Spec> {
    /// Standard Ethereum precompiles.
    eth_precompiles: EthPrecompiles,
    /// Set of enabled custom precompile addresses (loaded from state).
    enabled_addresses: Vec<Address>,
    /// Bank module for balance queries.
    bank_module: &'a sov_bank::Bank<S>,
}

impl<S: Spec> std::fmt::Debug for SovPrecompiles<'_, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SovPrecompiles")
            .field("eth_precompiles", &self.eth_precompiles)
            .field("enabled_addresses", &self.enabled_addresses)
            .finish_non_exhaustive()
    }
}

impl<'a, S: Spec> SovPrecompiles<'a, S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Create a new SovPrecompiles instance.
    ///
    /// # Arguments
    ///
    /// * `enabled_addresses` - Set of addresses where custom precompiles are enabled
    /// * `bank_module` - Reference to the bank module for balance queries
    pub fn new(enabled_addresses: Vec<Address>, bank_module: &'a sov_bank::Bank<S>) -> Self {
        Self {
            eth_precompiles: EthPrecompiles::default(),
            enabled_addresses,
            bank_module,
        }
    }
}

impl<'a, S, CTX> PrecompileProvider<CTX> for SovPrecompiles<'a, S>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
    CTX: ContextTr,
    CTX::Db: PrecompileDb<S>,
{
    type Output = InterpreterResult;

    fn set_spec(
        &mut self,
        spec: <<CTX as ContextTr>::Cfg as revm::context_interface::Cfg>::Spec,
    ) -> bool {
        <EthPrecompiles as PrecompileProvider<CTX>>::set_spec(&mut self.eth_precompiles, spec)
    }

    fn run(&mut self, ctx: &mut CTX, inputs: &CallInputs) -> Result<Option<Self::Output>, String> {
        let address = &inputs.target_address;
        let gas_limit = inputs.gas_limit;

        // Check if this precompile is enabled
        if self.enabled_addresses.contains(address) {
            let input_bytes = inputs.input.bytes(ctx);
            match *address {
                IDENTITY_PRECOMPILE_ADDRESS => {
                    let result = identity_precompile(&input_bytes, gas_limit);
                    return Ok(Some(convert_to_interpreter_result(result, gas_limit)));
                }
                BANK_BALANCE_PRECOMPILE_ADDRESS => {
                    // Access state through the context's database
                    let state = ctx.db_mut().precompile_state_mut();
                    let result = bank_balance_precompile::<S, _>(
                        &input_bytes,
                        gas_limit,
                        self.bank_module,
                        state,
                    );
                    return Ok(Some(convert_to_interpreter_result(result, gas_limit)));
                }
                _ => {
                    // Unknown precompile - should not happen since is_known_sov_precompile
                    // and the match should be consistent
                    return Err(format!("Precompile at {} not implemented", address));
                }
            }
        }

        // Fall back to standard Ethereum precompiles
        self.eth_precompiles.run(ctx, inputs)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        // Include both standard Ethereum precompile addresses and enabled custom addresses
        Box::new(
            self.eth_precompiles
                .warm_addresses()
                .chain(self.enabled_addresses.iter().copied()),
        )
    }

    fn contains(&self, address: &Address) -> bool {
        // Check if it's a standard Ethereum precompile OR an enabled sovereign precompile
        self.eth_precompiles.contains(address) || self.enabled_addresses.contains(address)
    }
}

/// Convert a PrecompileResult to an InterpreterResult.
fn convert_to_interpreter_result(result: PrecompileResult, gas_limit: u64) -> InterpreterResult {
    match result {
        Ok(output) => {
            let mut gas = Gas::new(gas_limit);
            let _ = gas.record_cost(output.gas_used);
            InterpreterResult {
                result: InstructionResult::Return,
                gas,
                output: output.bytes,
            }
        }
        Err(PrecompileError::OutOfGas) => InterpreterResult {
            result: InstructionResult::PrecompileOOG,
            gas: Gas::new(gas_limit),
            output: Bytes::new(),
        },
        Err(PrecompileError::Error(_msg)) => InterpreterResult {
            result: InstructionResult::PrecompileError,
            gas: Gas::new(gas_limit),
            output: Bytes::new(),
        },
    }
}

// =============================================================================
// Precompile Implementations
// =============================================================================

/// Gas cost constants for the identity precompile.
const IDENTITY_BASE_GAS: u64 = 15;
const IDENTITY_PER_WORD_GAS: u64 = 3;

/// Gas cost for the bank balance precompile (fixed cost for state read).
const BANK_BALANCE_GAS: u64 = 100;

/// Identity precompile - returns input unchanged.
///
/// This is a simple test precompile that demonstrates the stateful precompile
/// infrastructure. It simply returns its input bytes unchanged.
///
/// Gas cost: 15 base + 3 per 32-byte word (matching EIP-198 style).
///
/// # Arguments
///
/// * `input` - The input bytes to return
/// * `gas_limit` - Maximum gas available for this call
///
/// # Returns
///
/// `PrecompileResult` containing the input bytes unchanged, or an error if
/// the gas limit is exceeded.
fn identity_precompile(input: &Bytes, gas_limit: u64) -> PrecompileResult {
    // Calculate gas cost: base + per-word cost
    let word_count = (input.len() as u64 + 31) / 32;
    let gas_used = IDENTITY_BASE_GAS + IDENTITY_PER_WORD_GAS * word_count;

    if gas_used > gas_limit {
        return Err(PrecompileError::OutOfGas);
    }

    Ok(PrecompileOutput {
        gas_used,
        bytes: input.clone(),
    })
}

/// Bank balance precompile - returns the balance of an address for a token.
///
/// Input format:
/// - Bytes 0-19: 20-byte Ethereum address to query
/// - Bytes 20-51 (optional): 32-byte token ID (defaults to gas token if not provided)
///
/// Output: 32-byte U256 balance (big-endian)
///
/// Gas cost: 100 (fixed cost for state read)
///
/// # Arguments
///
/// * `input` - The input bytes containing address and optional token ID
/// * `gas_limit` - Maximum gas available for this call
/// * `bank_module` - Reference to the bank module for balance queries
/// * `state` - Mutable reference to the sovereign TxState
///
/// # Returns
///
/// `PrecompileResult` containing the balance as 32-byte U256, or an error.
fn bank_balance_precompile<S: Spec, Ws: TxState<S>>(
    input: &Bytes,
    gas_limit: u64,
    bank_module: &sov_bank::Bank<S>,
    state: &mut Ws,
) -> PrecompileResult
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    // Check gas limit
    if BANK_BALANCE_GAS > gas_limit {
        return Err(PrecompileError::OutOfGas);
    }

    let (address_bytes, token_id) = match input.len() {
        20 => {
            // Use default gas token
            (&input[0..20], config_gas_token_id())
        }
        52 => {
            // Token ID provided
            let mut token_bytes = [0u8; 32];
            token_bytes.copy_from_slice(&input[20..52]);
            (&input[0..20], TokenId::from(token_bytes))
        }
        _ => {
            return Err(PrecompileError::Error(
                "Input must be 20 or 52 bytes".to_string(),
            ));
        }
    };

    let address =  S::Address::from_vm_address(EthereumAddress::try_from(address_bytes).expect("Conversion from 20-byte slice to EthereumAddress is infallible"));
    // Query balance from bank module
    let balance = bank_module
        .get_balance_of(&address, token_id, state)
        .map_err(|e| PrecompileError::Error(format!("State error: {:?}", e)))?
        .unwrap_or_default();

    // Convert balance (u128) to U256 and encode as 32 bytes (big-endian)
    let balance_u256 = U256::from(balance.0);
    let output = Bytes::copy_from_slice(&balance_u256.to_be_bytes::<32>());

    Ok(PrecompileOutput {
        gas_used: BANK_BALANCE_GAS,
        bytes: output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_known_sov_precompile_identity() {
        assert!(is_known_sov_precompile(&IDENTITY_PRECOMPILE_ADDRESS));
    }

    #[test]
    fn test_is_known_sov_precompile_bank_balance() {
        assert!(is_known_sov_precompile(&BANK_BALANCE_PRECOMPILE_ADDRESS));
    }

    #[test]
    fn test_is_known_sov_precompile_unknown() {
        let unknown_addr = Address::from_slice(&[0u8; 20]);
        assert!(!is_known_sov_precompile(&unknown_addr));
    }

    #[test]
    fn test_identity_precompile_returns_input() {
        let input = Bytes::from_static(b"hello world");
        let result = identity_precompile(&input, 1000);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.bytes, input);
    }

    #[test]
    fn test_identity_precompile_gas_calculation() {
        // Empty input: 15 base + 3 * 0 words = 15
        let empty = Bytes::new();
        let result = identity_precompile(&empty, 1000).unwrap();
        assert_eq!(result.gas_used, 15);

        // 32 bytes: 15 base + 3 * 1 word = 18
        let one_word = Bytes::from(vec![0u8; 32]);
        let result = identity_precompile(&one_word, 1000).unwrap();
        assert_eq!(result.gas_used, 18);

        // 33 bytes: 15 base + 3 * 2 words = 21
        let two_words = Bytes::from(vec![0u8; 33]);
        let result = identity_precompile(&two_words, 1000).unwrap();
        assert_eq!(result.gas_used, 21);
    }

    #[test]
    fn test_identity_precompile_out_of_gas() {
        let input = Bytes::from(vec![0u8; 100]);
        // 100 bytes = 4 words, gas = 15 + 3*4 = 27
        let result = identity_precompile(&input, 20);

        assert!(matches!(result, Err(PrecompileError::OutOfGas)));
    }
}
