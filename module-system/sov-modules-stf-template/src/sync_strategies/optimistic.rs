use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::{Context, DispatchCall, Spec};
use sov_rollup_interface::optimistic::{Attestation, Challenge};
use sov_state::Storage;

pub struct OptimisticSyncRuntime<C: Context> {
    phantom: std::marker::PhantomData<C>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]

pub enum SyncMessage<'a, C: Context> {
    Attestation(Attestation<<<C as Spec>::Storage as Storage>::Proof>),
    #[serde(borrow)]
    Challenge(Challenge<'a>),
}

impl<C: Context> DispatchCall for OptimisticSyncRuntime<C> {
    type Context = C;

    type Decodable = ();

    fn decode_call(serialized_message: &[u8]) -> Result<Self::Decodable, std::io::Error> {
        todo!()
    }

    fn dispatch_call(
        &self,
        message: Self::Decodable,
        working_set: &mut sov_state::WorkingSet<
            <<Self as DispatchCall>::Context as sov_modules_api::Spec>::Storage,
        >,
        context: &Self::Context,
    ) -> Result<sov_modules_api::CallResponse, sov_modules_api::Error> {
        todo!()
    }

    fn module_address(
        &self,
        message: &Self::Decodable,
    ) -> &<Self::Context as sov_modules_api::Spec>::Address {
        todo!()
    }
}
