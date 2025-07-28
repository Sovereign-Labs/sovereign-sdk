use std::marker::PhantomData;

use sov_modules_api::{Amount, DaSpec, Gas, ProofReceipt, Spec, StateCheckpoint, Storage};
use sov_rollup_interface::stf::StoredEvent;
use sov_state::StorageProof;

use crate::proof_processing::process_proof;
use crate::Runtime;
/// An implementation of the
/// [`StateTransitionFunction`](sov_rollup_interface::stf::StateTransitionFunction)
/// that is specifically designed to work with the module-system.
pub struct StfBlueprint<S: Spec, RT: Runtime<S>> {
    /// The runtime includes all the modules that the rollup supports.
    #[cfg_attr(not(feature = "test-utils"), allow(dead_code))]
    pub(crate) runtime: RT,
    phantom_context: PhantomData<S>,
}

impl<S, RT> Default for StfBlueprint<S, RT>
where
    S: Spec,
    RT: Runtime<S>,
{
    fn default() -> Self {
        Self {
            runtime: RT::default(),
            phantom_context: PhantomData,
        }
    }
}

impl<S, RT> StfBlueprint<S, RT>
where
    S: Spec,
    RT: Runtime<S>,
{
    /// [`StfBlueprint`] constructor with the default [`Runtime`] value. Same as
    /// [`Default::default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// [`StfBlueprint`] constructor with a custom [`Runtime`] value.
    pub fn with_runtime(runtime: RT) -> Self {
        Self {
            runtime,
            ..Default::default()
        }
    }

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    #[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
    pub(crate) fn process_proof(
        &self,
        runtime: &mut RT,
        blob_hash: [u8; 32],
        slot_gas: &S::Gas,
        sender: &<S::Da as DaSpec>::Address,
        sequencer_rollup_address: &S::Address,
        sequencer_bond: Amount,
        gas_price: &<S::Gas as Gas>::Price,
        raw_proof: Vec<u8>,
        checkpoint: StateCheckpoint<S>,
    ) -> (
        ProofReceipt<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
            StorageProof<<S::Storage as Storage>::Proof>,
        >,
        StateCheckpoint<S>,
        S::Gas,
    ) {
        let (res, state) = process_proof(
            runtime,
            slot_gas,
            blob_hash,
            sender,
            sequencer_rollup_address,
            sequencer_bond,
            gas_price,
            raw_proof,
            checkpoint,
        );

        (res.proof_receipt, state, res.gas_used)
    }
}

#[cfg(feature = "native")]
pub(crate) fn convert_to_runtime_events<S, RT>(
    events: Vec<sov_modules_api::TypeErasedEvent>,
    tx_hash: [u8; 32],
) -> Vec<StoredEvent>
where
    S: Spec,
    RT: Runtime<S>,
{
    events
        .into_iter()
        .map(|typed_event| {
            // This seems to be needed because doing `&typed_event.event_key().to_vec()`
            // directly as the first function param to Event::new() is running into a linter bug
            // where it thinks that the to_vec is not necessary.
            // (probably due to the borrow and move in the same statement)
            // https://github.com/rust-lang/rust-clippy/issues/12098
            let key = typed_event.event_key().to_vec();
            StoredEvent::new(
                &key,
                &borsh::to_vec(
                    &<RT as sov_modules_api::RuntimeEventProcessor>::convert_to_runtime_event(
                        typed_event,
                    )
                    .expect("Unknown event type"),
                )
                .expect("unable to serialize event"),
                tx_hash,
            )
        })
        .collect()
}

#[cfg(not(feature = "native"))]
pub(crate) fn convert_to_runtime_events<S, RT>(
    _events: Vec<sov_modules_api::TypeErasedEvent>,
    _tx_hash: [u8; 32],
) -> Vec<StoredEvent>
where
    S: Spec,
    RT: Runtime<S>,
{
    Vec::new() // Return an empty vector
}
