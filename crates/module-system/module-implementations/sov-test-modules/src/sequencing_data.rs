use anyhow::{bail, ensure, Context as _};
use chrono::{TimeZone, Utc};
use sov_modules_api::{Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, Spec, TxState};

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
    type CallMessage = ();
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
        _msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        let data = context
            .sequencing_data()
            .as_ref()
            .context("No sequencing data in context")?;
        let timestamp = parse_timestamp(data)?;
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

fn parse_timestamp(data: &[u8]) -> anyhow::Result<u128> {
    let Ok(bytes) = data.try_into() else {
        bail!("Failed to convert to [u8; 16]");
    };
    Ok(u128::from_le_bytes(bytes))
}

fn year_to_timestamp(year: i32) -> u128 {
    Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0)
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap() as u128
}
