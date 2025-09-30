use alloy::{network::Network, providers::Provider};
use alloy_primitives::{Address, U256};
use anyhow::Result;
use sov_eth_client::TestClient;
use sov_test_utils::{Erc20, Router, Submit};

struct Contracts {
    weth: Address,
    usdc: Address,
    router: Address,
    _pair: Address,
}

async fn deploy_uniswap_contracts<P, N>(
    client: &P,
) -> Result<Contracts>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    let weth = Erc20::deploy(client, "Weth".into(), "WETH".into()).await?;
    let usdc = Erc20::deploy(client, "Usdc".into(), "USDC".into()).await?;
    let router = Router::deploy(client).await?;
    
    router
        .createPair(*weth.address(), *usdc.address())
        .submit()
        .await?;
    
    let pair_address = router
        .pairFor(*weth.address(), *usdc.address())
        .call()
        .await?;
    
    Ok(Contracts {
        weth: *weth.address(),
        usdc: *usdc.address(),
        router: *router.address(),
        _pair: pair_address,
    })
}

async fn add_initial_liquidity<P, N>(
    client: &P,
    contracts: &Contracts,
    signer: Address,
    weth_amount: U256,
    usdc_amount: U256,
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    let weth = Erc20::new(contracts.weth, client);
    let usdc = Erc20::new(contracts.usdc, client);
    let router = Router::new(contracts.router, client);
    
    // Mint and approve WETH
    weth.mint(signer, weth_amount).submit().await?;
    weth.approve(contracts.router, weth_amount)
        .submit()
        .await?;

    // Mint and approve USDC
    usdc.mint(signer, usdc_amount).submit().await?;
    usdc.approve(contracts.router, usdc_amount)
        .submit()
        .await?;

    // Add liquidity to the pool
    router
        .addLiquidity(
            contracts.weth,
            contracts.usdc,
            weth_amount,
            usdc_amount,
        )
        .submit()
        .await?;
    
    Ok(())
}

async fn execute_swap<P, N>(
    client: &P,
    contracts: &Contracts,
    signer: Address,
    amount_in: U256,
    from_token_address: Address,
    path: Vec<Address>,
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    let router = Router::new(contracts.router, client);
    let from_token = Erc20::new(from_token_address, client);
    
    // Calculate expected output using router's getAmountsOut
    let amounts_out = router
        .getAmountsOut(amount_in, path.clone())
        .call()
        .await?;
    let expected_out = amounts_out[1];
    
    println!(
        "Swapping {} tokens",
        amount_in / U256::from(10).pow(U256::from(18))
    );
    println!(
        "Expected output: {} tokens",
        expected_out / U256::from(10).pow(U256::from(18))
    );

    // Mint and approve tokens for swap
    from_token.mint(signer, amount_in).submit().await?;
    from_token
        .approve(contracts.router, amount_in)
        .submit()
        .await?;

    // Execute the swap with exact expected amount (no slippage)
    router
        .swapExactTokensForTokens(
            amount_in,
            expected_out,
            path,
            signer,
        )
        .submit()
        .await?;
    
    Ok(())
}

pub async fn run(client: TestClient) -> Result<()> {
    let signer = Address::from_slice(client.address().as_bytes());
    let client = &client.alloy_client;
    
    // Deploy all contracts
    let contracts = deploy_uniswap_contracts(client).await?;
    
    // Add initial liquidity: 1000 WETH : 2000 USDC (1 WETH = 2 USDC)
    let weth_liquidity = U256::from(1000) * U256::from(10).pow(U256::from(18));
    let usdc_liquidity = U256::from(2000) * U256::from(10).pow(U256::from(18));
    add_initial_liquidity(client, &contracts, signer, weth_liquidity, usdc_liquidity).await?;
    
    // Execute swap: 10 USDC for WETH
    let swap_amount = U256::from(10) * U256::from(10).pow(U256::from(18));
    let swap_path = vec![contracts.usdc, contracts.weth];
    execute_swap(client, &contracts, signer, swap_amount, contracts.usdc, swap_path).await?;
    
    Ok(())
}