use alloy_primitives::{Bytes, U256};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_bank::{config_gas_token_id, Bank, TokenId};
use sov_modules_api::{Spec, TxState};

use super::{
    Address, EvmPrecompile, EvmPrecompileEnv, EvmPrecompileSet, PrecompileError, PrecompileOutput,
    PrecompileResult,
};

/// The gas-token bank balance precompile address.
pub const BANK_BALANCE_PRECOMPILE_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00, 0x00,
]);

const BANK_BALANCE_GAS: u64 = 100;

/// A built-in precompile that returns a `sov-bank` token balance.
#[derive(Clone)]
pub struct BankBalancePrecompile<S: Spec> {
    bank: Bank<S>,
}

impl<S: Spec> Default for BankBalancePrecompile<S> {
    fn default() -> Self {
        Self {
            bank: Bank::default(),
        }
    }
}

impl<S> EvmPrecompile<S> for BankBalancePrecompile<S>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    const ADDRESS: Address = BANK_BALANCE_PRECOMPILE_ADDRESS;

    fn execute<ST: TxState<S>>(
        &self,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        bank_balance_precompile(input, gas_limit, &self.bank, env.state)
    }
}

impl<S> EvmPrecompileSet<S> for BankBalancePrecompile<S>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    const ADDRESSES: &'static [Address] = &[BANK_BALANCE_PRECOMPILE_ADDRESS];

    fn execute<ST: TxState<S>>(
        &self,
        _address: Address,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        <Self as EvmPrecompile<S>>::execute(self, input, gas_limit, env)
    }
}

fn bank_balance_precompile<S: Spec, ST: TxState<S>>(
    input: &[u8],
    gas_limit: u64,
    bank: &Bank<S>,
    state: &mut ST,
) -> PrecompileResult
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    if BANK_BALANCE_GAS > gas_limit {
        return Err(PrecompileError::OutOfGas);
    }

    let (address_bytes, token_id) = match input.len() {
        20 => {
            let mut address_bytes = [0u8; 20];
            address_bytes.copy_from_slice(&input[0..20]);
            (address_bytes, config_gas_token_id())
        }
        52 => {
            let mut address_bytes = [0u8; 20];
            address_bytes.copy_from_slice(&input[0..20]);
            let mut token_bytes = [0u8; 32];
            token_bytes.copy_from_slice(&input[20..52]);
            (address_bytes, TokenId::from(token_bytes))
        }
        _ => {
            return Err(PrecompileError::InvalidInput(format!(
                "expected 20-byte address or 20-byte address plus 32-byte token id, got {} bytes",
                input.len()
            )));
        }
    };

    let address = S::Address::from_vm_address(EthereumAddress::new(address_bytes));

    let balance = bank
        .get_balance_of(&address, token_id, state)
        .map_err(|e| PrecompileError::State(e.to_string()))?
        .unwrap_or_default();

    Ok(PrecompileOutput {
        gas_used: BANK_BALANCE_GAS,
        bytes: Bytes::copy_from_slice(&U256::from(balance.0).to_be_bytes::<32>()),
    })
}
