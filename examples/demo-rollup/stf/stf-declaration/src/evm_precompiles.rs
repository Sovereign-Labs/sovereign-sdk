use sov_address::{EthereumAddress, FromVmAddress};
use sov_evm::precompiles::{
    Address, BankBalancePrecompile, EvmPrecompileEnv, EvmPrecompileSet, PrecompileResult,
    SequencingTimestampPrecompile,
};
use sov_modules_api::{Spec, TxState};

#[derive(Clone)]
pub struct DemoEvmPrecompiles<S: Spec> {
    bank_balance: BankBalancePrecompile<S>,
    sequencing_timestamp: SequencingTimestampPrecompile<S>,
}

impl<S: Spec> Default for DemoEvmPrecompiles<S> {
    fn default() -> Self {
        Self {
            bank_balance: BankBalancePrecompile::default(),
            sequencing_timestamp: SequencingTimestampPrecompile::default(),
        }
    }
}

impl<S> EvmPrecompileSet<S> for DemoEvmPrecompiles<S>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn addresses(&self) -> impl Iterator<Item = Address> {
        self.bank_balance
            .addresses()
            .chain(self.sequencing_timestamp.addresses())
    }

    fn execute<ST: TxState<S>>(
        &self,
        address: Address,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> Option<PrecompileResult> {
        self.bank_balance
            .execute(address, input, gas_limit, env)
            .or_else(|| {
                self.sequencing_timestamp
                    .execute(address, input, gas_limit, env)
            })
    }
}
