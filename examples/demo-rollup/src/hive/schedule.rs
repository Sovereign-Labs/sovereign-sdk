use anyhow::Result;
use serde_json::{Map, Value};

use crate::parse::{ensure_object_field, parse_optional_u64};
use crate::rlp_chain::activation_block_for_timestamp;

fn push_block_fork(
    schedule: &mut Vec<(u64, &'static str)>,
    config: &Map<String, Value>,
    field: &str,
    fork_name: &'static str,
) -> Result<()> {
    if let Some(block) = parse_optional_u64(config.get(field), field)? {
        schedule.push((block, fork_name));
    }
    Ok(())
}

fn push_time_fork(
    schedule: &mut Vec<(u64, &'static str)>,
    config: &Map<String, Value>,
    field: &str,
    fork_name: &'static str,
    chain_timestamps: &[(u64, u64)],
) -> Result<()> {
    if let Some(timestamp) = parse_optional_u64(config.get(field), field)? {
        if let Some(block) = activation_block_for_timestamp(chain_timestamps, timestamp) {
            schedule.push((block, fork_name));
        }
    }
    Ok(())
}

fn build_hardfork_schedule(
    config: &Map<String, Value>,
    chain_timestamps: &[(u64, u64)],
) -> Result<Vec<(u64, &'static str)>> {
    let mut schedule: Vec<(u64, &'static str)> = vec![(0, "FRONTIER")];

    push_block_fork(&mut schedule, config, "homesteadBlock", "HOMESTEAD")?;
    push_block_fork(&mut schedule, config, "eip150Block", "TANGERINE")?;

    let eip155 = parse_optional_u64(config.get("eip155Block"), "eip155Block")?;
    let eip158 = parse_optional_u64(config.get("eip158Block"), "eip158Block")?;
    if let Some(spurious_block) = [eip155, eip158].into_iter().flatten().max() {
        schedule.push((spurious_block, "SPURIOUS_DRAGON"));
    }

    push_block_fork(&mut schedule, config, "byzantiumBlock", "BYZANTIUM")?;
    push_block_fork(
        &mut schedule,
        config,
        "constantinopleBlock",
        "CONSTANTINOPLE",
    )?;
    push_block_fork(&mut schedule, config, "petersburgBlock", "PETERSBURG")?;
    push_block_fork(&mut schedule, config, "istanbulBlock", "ISTANBUL")?;
    push_block_fork(&mut schedule, config, "muirGlacierBlock", "MUIR_GLACIER")?;
    push_block_fork(&mut schedule, config, "berlinBlock", "BERLIN")?;
    push_block_fork(&mut schedule, config, "londonBlock", "LONDON")?;
    push_block_fork(&mut schedule, config, "arrowGlacierBlock", "ARROW_GLACIER")?;
    push_block_fork(&mut schedule, config, "grayGlacierBlock", "GRAY_GLACIER")?;
    push_block_fork(&mut schedule, config, "mergeNetsplitBlock", "MERGE")?;

    push_time_fork(
        &mut schedule,
        config,
        "shanghaiTime",
        "SHANGHAI",
        chain_timestamps,
    )?;
    push_time_fork(
        &mut schedule,
        config,
        "cancunTime",
        "CANCUN",
        chain_timestamps,
    )?;
    push_time_fork(
        &mut schedule,
        config,
        "pragueTime",
        "PRAGUE",
        chain_timestamps,
    )?;

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
    geth_genesis: &Value,
    chain_timestamps: &[(u64, u64)],
) -> Result<()> {
    let Some(config) = geth_genesis.get("config").and_then(Value::as_object) else {
        return Ok(());
    };

    let hardforks = build_hardfork_schedule(config, chain_timestamps)?
        .into_iter()
        .map(|(block, fork)| {
            Value::Array(vec![Value::from(block), Value::String(fork.to_string())])
        })
        .collect::<Vec<_>>();
    let chain_spec = ensure_object_field(evm_genesis, "chain_spec")?;
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
        let geth_genesis = json!({
            "config": {
                "homesteadBlock": "0x1",
                "eip150Block": "0x1",
                "eip155Block": "0x2",
                "eip158Block": "0x3",
                "shanghaiTime": "0x64"
            }
        });
        let chain_timestamps = vec![(3, 90), (4, 100)];

        set_hardfork_schedule(&mut evm_genesis, &geth_genesis, &chain_timestamps).unwrap();

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
        set_hardfork_schedule(&mut evm_genesis, &json!({}), &[]).unwrap();
        assert!(evm_genesis.get("chain_spec").is_none());
    }
}
