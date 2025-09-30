use alloy::{network::Network, providers::Provider};
use alloy_primitives::{utils::{format_ether, parse_ether}, Address, U256};
use anyhow::Result;
use sov_eth_client::TestClient;
use sov_test_utils::{Erc20, Pair, Router, Submit};

struct Contracts<'a, P, N> 
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    weth: Erc20::Erc20Instance<&'a P, N>,
    usdc: Erc20::Erc20Instance<&'a P, N>,
    router: Router::RouterInstance<&'a P, N>,
    _pair: Pair::PairInstance<&'a P, N>,
}

pub async fn run(client: TestClient) -> Result<()> {
    let signer = Address::from_slice(client.address().as_bytes());
    let client = &client.alloy_client;
    
    let contracts = deploy_uniswap_contracts(client).await?;
    
    let weth_liquidity = parse_ether("1000")?;
    let usdc_liquidity = parse_ether("2000")?;
    add_initial_liquidity(&contracts, signer, weth_liquidity, usdc_liquidity).await?;
    
    let swap_amount = parse_ether("10")?;
    let swap_path = vec![*contracts.usdc.address(), *contracts.weth.address()];
    execute_swap(&contracts, signer, swap_amount, swap_path).await?;
    
    Ok(())
}

async fn execute_swap<P, N>(
    contracts: &Contracts<'_, P, N>,
    signer: Address,
    amount_in: U256,
    path: Vec<Address>,
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    let expected_out = contracts.router.getAmountsOut(amount_in, path.clone()).call().await?[1];
    println!("Swapping {} -> {} ETH", format_ether(amount_in), format_ether(expected_out));

    let from_token = if path[0] == *contracts.usdc.address() {
        &contracts.usdc
    } else {
        &contracts.weth
    };

    mint_and_approve(from_token, signer, *contracts.router.address(), amount_in).await?;
    contracts.router.swapExactTokensForTokens(amount_in, expected_out, path, signer).submit().await?;
    
    Ok(())
}

async fn add_initial_liquidity<P, N>(
    contracts: &Contracts<'_, P, N>,
    signer: Address,
    weth_amount: U256,
    usdc_amount: U256,
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    mint_and_approve(&contracts.weth, signer, *contracts.router.address(), weth_amount).await?;
    mint_and_approve(&contracts.usdc, signer, *contracts.router.address(), usdc_amount).await?;

    contracts.router
        .addLiquidity(
            *contracts.weth.address(),
            *contracts.usdc.address(),
            weth_amount,
            usdc_amount,
        )
        .submit()
        .await?;
    
    Ok(())
}

async fn deploy_uniswap_contracts<P, N>(
    client: &P,
) -> Result<Contracts<'_, P, N>>
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
    let pair = Pair::new(pair_address, client);
    
    Ok(Contracts {
        weth,
        usdc,
        router,
        _pair: pair,
    })
}

async fn mint_and_approve<P, N>(
    token: &Erc20::Erc20Instance<P, N>, 
    to: Address, 
    spender: Address, 
    amount: U256
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    token.mint(to, amount).submit().await?;
    token.approve(spender, amount).submit().await?;
    Ok(())
}