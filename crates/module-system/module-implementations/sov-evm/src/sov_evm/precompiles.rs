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
//! - `is_known_precompile`: Function to check if an address has a precompile implementation
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
//! 2. Add a match arm in `is_known_precompile` for the address
//! 3. Add a match arm in `invoke_precompile` that calls your implementation
//! 4. Implement the precompile logic (can access TxState)

use alloy_primitives::{Address, Bytes};
use revm::context_interface::ContextTr;
use revm::handler::{EthPrecompiles, PrecompileProvider};
use revm::interpreter::{CallInputs, Gas, InstructionResult, InterpreterResult};
use revm::primitives::hardfork::SpecId;
use sov_modules_api::{Context, Spec, TxState};
use std::collections::HashSet;

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

/// Identity precompile address (0x0100).
///
/// This is a test precompile that returns its input unchanged.
/// Gas cost: 15 base + 3 per word (matching EIP-198 ecrecover style).
pub const IDENTITY_PRECOMPILE_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00,
]);

/// Check if an address has a known precompile implementation in code.
///
/// Precompiles are only callable if they are both:
/// 1. Known (this function returns true)
/// 2. Enabled in the `enabled_precompiles` state (activation via admin tx)
///
/// When adding a new precompile, add a match arm here for its address.
pub fn is_known_precompile(address: &Address) -> bool {
    matches!(
        *address,
        IDENTITY_PRECOMPILE_ADDRESS
        // Add new precompile addresses here:
        // | BRIDGE_PRECOMPILE_ADDRESS
        // | CROSS_MODULE_CALL_ADDRESS
    )
}

/// PrecompileProvider that supports both standard Ethereum precompiles
/// and custom stateful sovereign precompiles.
///
/// This struct is created fresh for each transaction execution and holds
/// references to the enabled precompile addresses and sovereign context.
/// Stateful precompiles are invoked via the `invoke_precompile` method which
/// receives direct access to `TxState`.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SovPrecompiles<'a, S: Spec> {
    /// Standard Ethereum precompiles.
    eth_precompiles: EthPrecompiles,
    /// Set of enabled custom precompile addresses (loaded from state).
    enabled_addresses: HashSet<Address>,
    /// Sovereign context for sender info, execution context, etc.
    sov_context: &'a Context<S>,
    /// Current EVM specification.
    spec: SpecId,
}

impl<'a, S: Spec> SovPrecompiles<'a, S> {
    /// Create a new SovPrecompiles instance.
    ///
    /// # Arguments
    ///
    /// * `enabled_addresses` - Set of addresses where custom precompiles are enabled
    /// * `sov_context` - Sovereign SDK context for the current transaction
    #[allow(dead_code)]
    pub fn new(enabled_addresses: HashSet<Address>, sov_context: &'a Context<S>) -> Self {
        Self {
            eth_precompiles: EthPrecompiles::default(),
            enabled_addresses,
            sov_context,
            spec: SpecId::CANCUN,
        }
    }

    /// Invoke a stateful precompile with access to TxState.
    ///
    /// This method dispatches to the appropriate precompile implementation based on
    /// the target address. Precompile implementations have full access to sovereign
    /// state through the `state` parameter.
    ///
    /// # Arguments
    ///
    /// * `address` - The precompile address being called
    /// * `input` - The input bytes to the precompile
    /// * `gas_limit` - Maximum gas available for this call
    /// * `_state` - Mutable reference to the sovereign TxState (for stateful precompiles)
    ///
    /// # Returns
    ///
    /// `Some(PrecompileResult)` if the address matches a known precompile,
    /// `None` if the address is not a sovereign precompile.
    #[allow(dead_code)]
    pub fn invoke_precompile<Ws: TxState<S>>(
        &self,
        address: &Address,
        input: &Bytes,
        gas_limit: u64,
        _state: &mut Ws,
    ) -> Option<PrecompileResult> {
        // Match on the address to dispatch to the appropriate precompile.
        // Each match arm can access `_state` for stateful operations.
        match *address {
            IDENTITY_PRECOMPILE_ADDRESS => Some(identity_precompile(input, gas_limit)),
            // Future precompiles can be added here:
            // BRIDGE_PRECOMPILE_ADDRESS => Some(bridge_precompile(input, gas_limit, _state)),
            // CROSS_MODULE_CALL_ADDRESS => Some(cross_module_call(input, gas_limit, _state, self.sov_context)),
            _ => None,
        }
    }

    /// Check if an address is a known sovereign precompile (has implementation in code).
    #[allow(dead_code)]
    pub fn is_sovereign_precompile(address: &Address) -> bool {
        is_known_precompile(address)
    }
}

impl<'a, S, CTX> PrecompileProvider<CTX> for SovPrecompiles<'a, S>
where
    S: Spec,
    CTX: ContextTr,
{
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <<CTX as ContextTr>::Cfg as revm::context_interface::Cfg>::Spec) -> bool {
        <EthPrecompiles as PrecompileProvider<CTX>>::set_spec(&mut self.eth_precompiles, spec)
    }

    fn run(
        &mut self,
        ctx: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        let address = &inputs.target_address;
        let gas_limit = inputs.gas_limit;

        // Check if this is an enabled custom precompile
        if self.enabled_addresses.contains(address) {
            // Check if implementation exists in code
            if is_known_precompile(address) {
                // For now, we call precompiles without state access from the PrecompileProvider.
                // Full stateful precompile support requires calling invoke_precompile from
                // the executor with TxState access. This path handles the basic case.
                //
                // TODO: When integrating with executor, pass TxState through context's DB
                // and use invoke_precompile for full stateful access.
                let input_bytes = inputs.input.bytes(ctx);
                match *address {
                    IDENTITY_PRECOMPILE_ADDRESS => {
                        let result = identity_precompile(&input_bytes, gas_limit);
                        return Ok(Some(convert_to_interpreter_result(result, gas_limit)));
                    }
                    _ => {
                        // Unknown precompile - should not happen if is_known_precompile is correct
                        return Err(format!("Precompile at {} not implemented", address));
                    }
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
                .chain(self.enabled_addresses.iter().copied())
        )
    }

    fn contains(&self, address: &Address) -> bool {
        // Check if it's either a standard precompile or an enabled custom precompile
        self.eth_precompiles.contains(address)
            || (self.enabled_addresses.contains(address) && is_known_precompile(address))
    }
}

/// Convert a PrecompileResult to an InterpreterResult.
fn convert_to_interpreter_result(
    result: PrecompileResult,
    gas_limit: u64,
) -> InterpreterResult {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_known_precompile_identity() {
        assert!(is_known_precompile(&IDENTITY_PRECOMPILE_ADDRESS));
    }

    #[test]
    fn test_is_known_precompile_unknown() {
        let unknown_addr = Address::from_slice(&[0u8; 20]);
        assert!(!is_known_precompile(&unknown_addr));
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
