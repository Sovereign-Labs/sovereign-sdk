use std::marker::PhantomData;
use std::sync::Arc;

use sov_modules_api::capabilities::{
    AuthorizationData, ChainState, HasCapabilities, TransactionAuthorizer,
};
use sov_modules_api::prelude::*;
use sov_modules_api::rest::StateUpdateReceiver;
use sov_modules_api::transaction::{AuthenticatedTransactionData, TxDetails};
use sov_modules_api::{DaSpec, FullyBakedTx, Gas, GasArray, Spec, StateCheckpoint, WorkingSet};
use sov_modules_stf_blueprint::{apply_tx, ApplyTxResult, Runtime};
use sov_rollup_interface::common::HexString;

use crate::{PartialTransaction, RollupStateProvider};

/// The default rollup state provider. Uses the kernel and a runtime to simulate transaction execution and compute the gas price.
pub struct DefaultRollupStateProvider<S: Spec, RT: Runtime<S>> {
    phantom: PhantomData<(S, RT)>,
}

impl<S: Spec, RT: Runtime<S>> RollupStateProvider for Arc<DefaultRollupStateProvider<S, RT>>
where
    RT: HasCapabilities<S>,
{
    type Spec = S;

    type Runtime = RT;

    type Error = anyhow::Error;

    fn get_latest_base_fee_per_gas(
        state_update_receiver: &StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
    ) -> Result<<<Self::Spec as Spec>::Gas as Gas>::Price, Self::Error> {
        let storage = state_update_receiver.borrow().clone().storage;

        let mut state = StateCheckpoint::new(storage, &RT::default().kernel());

        RT::default().chain_state().base_fee_per_gas(&mut state).ok_or_else(|| anyhow::anyhow!("Impossible to get the base fee per gas for the current slot. The slot requested may be too far in the future and the base fee per gas is not yet known."))
    }

    fn simulate_execution(
        state_update_receiver: &StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
        default_sequencer: <<Self::Spec as Spec>::Da as DaSpec>::Address,
        default_sequencer_rollup_address: <Self::Spec as Spec>::Address,
        transaction: PartialTransaction<Self::Spec>,
    ) -> Result<ApplyTxResult<S>, Self::Error> {
        let auth_data =
            <PartialTransaction<S> as Into<AuthorizationData<S>>>::into(transaction.clone());
        let mut state = StateCheckpoint::new(
            state_update_receiver.borrow().storage.clone(),
            &RT::default().kernel(),
        );

        let gas_price = match transaction
            .gas_price {
            Some(gas_price) => gas_price,
            None => {
                match RT::default().chain_state().base_fee_per_gas(&mut state) {
                    Some(gas_price) => gas_price,
                    None => return Err(anyhow::anyhow!("Impossible to get the base fee per gas for the current slot. The slot requested may be too far in the future and the base fee per gas is not yet known."))
                }
            }
        };

        let sequencer_da_address = transaction.sequencer.unwrap_or(default_sequencer);
        let sequencer_rollup_address = transaction
            .sequencer_rollup_address
            .unwrap_or(default_sequencer_rollup_address);
        let mut runtime = RT::default();

        let tx_data = AuthenticatedTransactionData(TxDetails {
            chain_id: transaction.details.chain_id,
            max_priority_fee_bips: transaction.details.max_priority_fee_bips,
            max_fee: transaction.details.max_fee,
            gas_limit: transaction.details.gas_limit,
        });

        let mut scratchpad = state.to_tx_scratchpad();

        let decoded_call_message = RT::decode_call(&transaction.encoded_call_message)
            .map_err(|e| anyhow::anyhow!("Unable to deserialize call message: {e}"))?;

        let ctx = runtime.transaction_authorizer().resolve_context(
            &auth_data,
            &sequencer_da_address,
            sequencer_rollup_address,
            &mut scratchpad,
        )?;

        let working_set_gas_meter = tx_data.gas_meter(&gas_price.clone(), &<S::Gas>::MAX);

        let working_set =
            WorkingSet::create_working_set(scratchpad, &tx_data, working_set_gas_meter);

        let (result, _) = apply_tx(
            &mut runtime,
            &ctx,
            &tx_data,
            // We don't have a way to get the raw transaction hash because it depends on the signature.
            HexString::new([0; 32]),
            // We use an empty tx body because we can't build an actual one without the signature. We could make something *more* accurate here, but it might be confusing to the user.
            FullyBakedTx::new(vec![]),
            decoded_call_message,
            working_set,
        );

        Ok(result)
    }
}
