use alloy::{network::Network, providers::Provider};
use alloy_primitives::{utils::parse_ether, Address, U256};
use anyhow::Result;
use sov_eth_client::TestClient;
use sov_test_utils::{Erc20, Pair, Router, Submit};
use rand::Rng;

struct Contracts<'a, P, N> {
    weth: Erc20::Erc20Instance<&'a P, N>,
    usdc: Erc20::Erc20Instance<&'a P, N>,
    router: Router::RouterInstance<&'a P, N>,
    _pair: Pair::PairInstance<&'a P, N>,
}

pub async fn run(client: TestClient) -> Result<()> {
    let signer = Address::from_slice(client.address().as_bytes());
    let client = &client.alloy_client;
    
    println!("🚀 Deploying Uniswap contracts...");
    let contracts = deploy_uniswap_contracts(client).await?;
    
    // Larger pool for load testing - 10k WETH : 20k USDC  
    println!("💧 Adding liquidity: 10,000 WETH + 20,000 USDC");
    let weth_liquidity = parse_ether("10000")?;
    let usdc_liquidity = parse_ether("20000")?;
    add_initial_liquidity(&contracts, signer, weth_liquidity, usdc_liquidity).await?;
    
    println!("🔄 Starting load test: 100 random swaps...");
    execute_random_swaps(&contracts, signer, 100).await?;
    
    println!("✅ Load test completed successfully!");
    Ok(())
}

async fn execute_random_swaps<P, N>(
    contracts: &Contracts<'_, P, N>,
    signer: Address,
    count: usize,
) -> Result<()>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    let mut rng = rand::thread_rng();
    
    for i in 1..=count {
        // Alternate between USDC→WETH and WETH→USDC to keep pool balanced
        let is_usdc_to_weth = i % 2 == 1;
        
        let (path, base_amount) = if is_usdc_to_weth {
            (vec![*contracts.usdc.address(), *contracts.weth.address()], "20000") // Max 20k USDC
        } else {
            (vec![*contracts.weth.address(), *contracts.usdc.address()], "10000") // Max 10k WETH  
        };
        
        // Random amount between 0.1% and 5% of pool reserves
        let min_percent = 0.001; // 0.1%
        let max_percent = 0.05;  // 5%
        let random_percent = rng.gen_range(min_percent..=max_percent);
        
        let base_value = base_amount.parse::<f64>()?;
        let scaled_amount = format!("{:.6}", random_percent * base_value);
        let amount = parse_ether(&scaled_amount)?;
        
        execute_swap(contracts, signer, amount, path).await?;
        
        if i % 10 == 0 {
            println!("📊 Completed {}/{} swaps", i, count);
        }
    }
    
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