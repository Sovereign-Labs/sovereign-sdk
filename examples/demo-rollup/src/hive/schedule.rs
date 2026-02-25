use anyhow::Result;
use serde_json::Value;

use crate::rlp_chain::activation_block_for_timestamp;
use crate::types::{object_field_or_insert, GethConfig, U64Like};

fn as_u64(value: &Option<U64Like>, field_name: &str) -> Result<Option<u64>> {
    value.as_ref().map(|v| v.to_u64(field_name)).transpose()
}

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
    config: &GethConfig,
    chain_timestamps: &[(u64, u64)],
) -> Result<Vec<(u64, &'static str)>> {
    let mut schedule: Vec<(u64, &'static str)> = vec![(0, "FRONTIER")];

    push_block_fork(
        &mut schedule,
        as_u64(&config.homestead_block, "config.homesteadBlock")?,
        "HOMESTEAD",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.eip150_block, "config.eip150Block")?,
        "TANGERINE",
    );

    let eip155 = as_u64(&config.eip155_block, "config.eip155Block")?;
    let eip158 = as_u64(&config.eip158_block, "config.eip158Block")?;
    if let Some(spurious_block) = [eip155, eip158].into_iter().flatten().max() {
        schedule.push((spurious_block, "SPURIOUS_DRAGON"));
    }

    push_block_fork(
        &mut schedule,
        as_u64(&config.byzantium_block, "config.byzantiumBlock")?,
        "BYZANTIUM",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.constantinople_block, "config.constantinopleBlock")?,
        "CONSTANTINOPLE",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.petersburg_block, "config.petersburgBlock")?,
        "PETERSBURG",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.istanbul_block, "config.istanbulBlock")?,
        "ISTANBUL",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.muir_glacier_block, "config.muirGlacierBlock")?,
        "MUIR_GLACIER",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.berlin_block, "config.berlinBlock")?,
        "BERLIN",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.london_block, "config.londonBlock")?,
        "LONDON",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.arrow_glacier_block, "config.arrowGlacierBlock")?,
        "ARROW_GLACIER",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.gray_glacier_block, "config.grayGlacierBlock")?,
        "GRAY_GLACIER",
    );
    push_block_fork(
        &mut schedule,
        as_u64(&config.merge_netsplit_block, "config.mergeNetsplitBlock")?,
        "MERGE",
    );

    push_time_fork(
        &mut schedule,
        as_u64(&config.shanghai_time, "config.shanghaiTime")?,
        "SHANGHAI",
        chain_timestamps,
    );
    push_time_fork(
        &mut schedule,
        as_u64(&config.cancun_time, "config.cancunTime")?,
        "CANCUN",
        chain_timestamps,
    );
    push_time_fork(
        &mut schedule,
        as_u64(&config.prague_time, "config.pragueTime")?,
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

    Ok(deduped)
}

pub(crate) fn set_hardfork_schedule(
    evm_genesis: &mut Value,
    config: Option<&GethConfig>,
    chain_timestamps: &[(u64, u64)],
) -> Result<()> {
    let Some(config) = config else {
        return Ok(());
    };

    let hardforks = build_hardfork_schedule(config, chain_timestamps)?
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
        let config: GethConfig = serde_json::from_value(json!({
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
