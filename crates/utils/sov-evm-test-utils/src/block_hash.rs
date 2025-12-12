use alloy_sol_types::sol;

sol!(
    #[sol(
        rpc,
        all_derives = true,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/artifacts/", "BlockHash.bin")))]
    BlockHash,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/artifacts/",
        "BlockHash.abi"
    )
);
