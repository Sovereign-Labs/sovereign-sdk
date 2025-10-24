use alloy_sol_types::sol;

sol!(
    #[sol(
        rpc,
        all_derives = true,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/artifacts/", "ERC20.bin")))]
    Erc20,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/artifacts/",
        "ERC20.abi"
    )
);

sol!(
    #[sol(
        rpc,
        all_derives = true,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/artifacts/", "Router.bin")))]
    Router,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/artifacts/",
        "Router.abi"
    )
);
