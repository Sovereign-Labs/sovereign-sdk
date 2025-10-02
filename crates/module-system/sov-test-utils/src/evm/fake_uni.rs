use alloy_sol_types::sol;

sol!(
    #[sol(
        rpc,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/evm/test-data/artifacts/", "ERC20.bin")))]
    Erc20,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/evm/test-data/artifacts/",
        "ERC20.abi"
    )
);

sol!(
    #[sol(
        rpc,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/evm/test-data/artifacts/", "Router.bin")))]
    Router,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/evm/test-data/artifacts/",
        "Router.abi"
    )
);
