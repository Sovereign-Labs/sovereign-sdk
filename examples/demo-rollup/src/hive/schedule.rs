use alloy::genesis::ChainConfig;
use anyhow::Result;
use serde_json::Value;

use crate::rlp_chain::activation_block_for_timestamp;
use crate::types::object_field_or_insert;

fn push_block_fork(
    schedule: &mut Vec<(u64, &'static str)>,
    block: Option<u64>,
    fork_name: &'static str,
) {
    if let Some(block) = block {
        schedule.push((block, fork_name));
    }
}

fn push_time_fork(
    schedule: &mut Vec<(u64, &'static str)>,
    timestamp: Option<u64>,
    fork_name: &'static str,
    chain_timestamps: &[(u64, u64)],
) {
    if let Some(timestamp) = timestamp {
        if let Some(block) = activation_block_for_timestamp(chain_timestamps, timestamp) {
            schedule.push((block, fork_name));
        }
    }
}

fn build_hardfork_schedule(
    config: &ChainConfig,
    chain_timestamps: &[(u64, u64)],
) -> Vec<(u64, &'static str)> {
    let mut schedule: Vec<(u64, &'static str)> = vec![(0, "FRONTIER")];

    push_block_fork(&mut schedule, config.homestead_block, "HOMESTEAD");
    push_block_fork(&mut schedule, config.eip150_block, "TANGERINE");

    if let Some(spurious_block) = [config.eip155_block, config.eip158_block]
        .into_iter()
        .flatten()
        .max()
    {
        schedule.push((spurious_block, "SPURIOUS_DRAGON"));
    }

    push_block_fork(&mut schedule, config.byzantium_block, "BYZANTIUM");
    push_block_fork(&mut schedule, config.constantinople_block, "CONSTANTINOPLE");
    push_block_fork(&mut schedule, config.petersburg_block, "PETERSBURG");
    push_block_fork(&mut schedule, config.istanbul_block, "ISTANBUL");
    push_block_fork(&mut schedule, config.muir_glacier_block, "MUIR_GLACIER");
    push_block_fork(&mut schedule, config.berlin_block, "BERLIN");
    push_block_fork(&mut schedule, config.london_block, "LONDON");
    push_block_fork(&mut schedule, config.arrow_glacier_block, "ARROW_GLACIER");
    push_block_fork(&mut schedule, config.gray_glacier_block, "GRAY_GLACIER");
    push_block_fork(&mut schedule, config.merge_netsplit_block, "MERGE");

    push_time_fork(
        &mut schedule,
        config.shanghai_time,
        "SHANGHAI",
        chain_timestamps,
    );
    push_time_fork(
        &mut schedule,
        config.cancun_time,
        "CANCUN",
        chain_timestamps,
    );
    push_time_fork(
        &mut schedule,
        config.prague_time,
        "PRAGUE",
        chain_timestamps,
    );

    schedule.sort_by_key(|(block, _)| *block);
    let mut deduped: Vec<(u64, &'static str)> = Vec::with_capacity(schedule.len());
    for (block, fork_name) in schedule {
        if let Some((last_block, last_name)) = deduped.last_mut() {
            if *last_block == block {
                *last_name = fork_name;
                continue;
            }
        }
        deduped.push((block, fork_name));
    }

    deduped
}

pub(crate) fn set_hardfork_schedule(
    evm_genesis: &mut Value,
    config: Option<&ChainConfig>,
    chain_timestamps: &[(u64, u64)],
) -> Result<()> {
    let Some(config) = config else {
        return Ok(());
    };

    let hardforks = build_hardfork_schedule(config, chain_timestamps)
        .into_iter()
        .map(|(block, fork)| {
            Value::Array(vec![Value::from(block), Value::String(fork.to_string())])
        })
        .collect::<Vec<_>>();
    let chain_spec = object_field_or_insert(evm_genesis, "chain_spec")?;
    chain_spec.insert("hardforks".to_string(), Value::Array(hardforks));

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn set_hardfork_schedule_dedupes_same_block() {
        let mut evm_genesis = json!({});
        let config: ChainConfig = serde_json::from_value(json!({
            "homesteadBlock": "0x1",
            "eip150Block": "0x1",
            "eip155Block": "0x2",
            "eip158Block": "0x3",
            "shanghaiTime": "0x64"
        }))
        .unwrap();
        let chain_timestamps = vec![(3, 90), (4, 100)];

        set_hardfork_schedule(&mut evm_genesis, Some(&config), &chain_timestamps).unwrap();

        let hardforks = evm_genesis["chain_spec"]["hardforks"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(hardforks[0], json!([0, "FRONTIER"]));
        assert_eq!(hardforks[1], json!([1, "TANGERINE"]));
        assert_eq!(hardforks[2], json!([3, "SPURIOUS_DRAGON"]));
        assert_eq!(hardforks[3], json!([4, "SHANGHAI"]));
    }

    #[test]
    fn set_hardfork_schedule_no_config_is_noop() {
        let mut evm_genesis = json!({});
        set_hardfork_schedule(&mut evm_genesis, None, &[]).unwrap();
        assert!(evm_genesis.get("chain_spec").is_none());
    }
}
