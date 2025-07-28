use std::fs::File;
use std::io::{self, Write};

use schemars::schema_for;
use sov_mock_da::verifier::MockDaSpec;
use sov_mock_da::MockDaService;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::{ModuleCallJsonSchema, Spec};
use sov_rollup_interface::execution_mode;
use sov_stf_runner::RollupConfig;

type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, execution_mode::Native>;

fn main() -> io::Result<()> {
    // Call message schemas.
    store_module_call_json_schema::<sov_bank::Bank<S>>("sov-bank.json")?;
    store_module_call_json_schema::<sov_accounts::Accounts<S>>("sov-accounts.json")?;
    store_module_call_json_schema::<sov_value_setter::ValueSetter<S>>("sov-value-setter.json")?;
    store_module_call_json_schema::<sov_prover_incentives::ProverIncentives<S>>(
        "sov-prover-incentives.json",
    )?;
    store_module_call_json_schema::<sov_sequencer_registry::SequencerRegistry<S>>(
        "sov-sequencer-registry.json",
    )?;

    // Schemas for genesis configs.
    store_genesis_config_json_schema::<sov_bank::BankConfig<S>>("sov-bank.json")?;
    store_genesis_config_json_schema::<sov_accounts::AccountConfig<S>>("sov-accounts.json")?;
    store_genesis_config_json_schema::<sov_value_setter::ValueSetterConfig<S>>(
        "sov-value-setter.json",
    )?;
    store_genesis_config_json_schema::<sov_prover_incentives::ProverIncentivesConfig<S>>(
        "sov-prover-incentives.json",
    )?;
    store_genesis_config_json_schema::<sov_sequencer_registry::SequencerConfig<S>>(
        "sov-sequencer-registry.json",
    )?;

    // Rollup configuration schema.
    store_rollup_config_json_schema("rollup-config.json")?;

    Ok(())
}

fn store_rollup_config_json_schema(filename: &str) -> io::Result<()> {
    let schema = schema_for!(RollupConfig<<S as Spec>::Address, MockDaService>);
    let schema_string = serde_json::to_string_pretty(&schema)?;

    let mut file = File::create(filename)?;
    file.write_all(schema_string.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn store_genesis_config_json_schema<T: schemars::JsonSchema>(filename: &str) -> io::Result<()> {
    let schema = schema_for!(T);
    let schema_string = serde_json::to_string_pretty(&schema)?;

    let mut file = File::create(format!("genesis-schemas/{filename}"))?;
    file.write_all(schema_string.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn store_module_call_json_schema<M: ModuleCallJsonSchema>(filename: &str) -> io::Result<()> {
    let mut file = File::create(format!("schemas/{filename}"))?;
    file.write_all(M::json_schema().as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}
