use anyhow::ensure;
use borsh::{BorshDeserialize, BorshSerialize};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{
    Context, DaSpec, GenesisState, HDTimestamp, Module, ModuleId, ModuleInfo, Spec, TxState,
};

#[derive(
    Clone,
    BorshSerialize,
    BorshDeserialize,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub enum CallMessage {
    AssertTimestampIsReasonable,
    Noop,
}

#[derive(Clone, ModuleInfo)]
pub struct SequencingDataTester<S: Spec> {
    #[id]
    pub id: ModuleId,
    #[phantom]
    _phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for SequencingDataTester<S> {
    type Spec = S;

    type Config = ();
    type CallMessage = CallMessage;
    type Event = ();
    type Error = anyhow::Error;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        if matches!(msg, CallMessage::Noop) {
            return Ok(());
        }

        let timestamp = context
            .sequencing_data_view::<HDTimestamp>()
            .get(&())?
            .ok_or_else(|| anyhow::anyhow!("No sequencing data in context"))?
            .as_nanos();
        let reasonable_range = year_to_timestamp(2025)..year_to_timestamp(2100);
        ensure!(
            reasonable_range.contains(&timestamp),
            "Timestamp {} is outside reasonable range {:?}",
            timestamp,
            reasonable_range
        );
        Ok(())
    }
}

fn year_to_timestamp(year: i32) -> u128 {
    Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0)
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap() as u128
}
