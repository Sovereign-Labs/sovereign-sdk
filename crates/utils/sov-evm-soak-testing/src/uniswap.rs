use alloy::{network::Network, providers::Provider};
use alloy_primitives::{utils::parse_ether, Address, U256};
use anyhow::Result;
use rand::Rng;
use sov_test_utils::{Erc20, Router, Submit};

pub struct UniSoakTest<P, N> {
    weth: Erc20::Erc20Instance<P, N>,
    usdc: Erc20::Erc20Instance<P, N>,
    router: Router::RouterInstance<P, N>,
    signer: Address,
}

impl<P, N> UniSoakTest<P, N>
where
    P: Provider<N> + Clone + Send + Sync,
    N: Network + Send + Sync,
{
    pub async fn new(client: P, signer: Address) -> Result<Self> {
        println!("🚀 Deploying Uniswap contracts...");

        let weth = Erc20::deploy(client.clone(), "Weth".into(), "WETH".into()).await?;
        let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;

        let router = Router::deploy(client.clone()).await?;
        router
            .createPair(*weth.address(), *usdc.address())
            .submit()
            .await?;

        Ok(Self {
            signer,
            weth,
            usdc,
            router,
        })
    }

    pub async fn run(&self, count: usize) -> Result<()> {
        // Larger pool for load testing - 10k WETH : 20k USDC
        println!("💧 Adding liquidity: 10,000 WETH + 20,000 USDC");
        let weth_liquidity = parse_ether("10000")?;
        let usdc_liquidity = parse_ether("20000")?;
        self.add_initial_liquidity(weth_liquidity, usdc_liquidity)
            .await?;

        println!("🔄 Starting load test: {count} random swaps...");
        self.execute_random_swaps(count).await?;

        println!("✅ Load test completed successfully!");
        Ok(())
    }

    async fn execute_random_swaps(&self, count: usize) -> Result<()> {
        let mut rng = rand::thread_rng();

        for i in 1..=count {
            // Alternate between USDC→WETH and WETH→USDC to keep pool balanced
            let is_usdc_to_weth = i % 2 == 1;

            let (path, base_amount) = if is_usdc_to_weth {
                (vec![*self.usdc.address(), *self.weth.address()], "20000") // Max 20k USDC
            } else {
                (vec![*self.weth.address(), *self.usdc.address()], "10000") // Max 10k WETH
            };

            // Random amount between 0.1% and 5% of pool reserves
            let min_percent = 0.001; // 0.1%
            let max_percent = 0.05; // 5%
            let random_percent = rng.gen_range(min_percent..=max_percent);

            let base_value = base_amount.parse::<f64>()?;
            #[allow(clippy::float_arithmetic)] // This is a soak test and not guest code
            let scaled_amount = format!("{:.6}", random_percent * base_value);
            let amount = parse_ether(&scaled_amount)?;

            self.execute_swap(amount, path).await?;

            if i % 10 == 0 {
                println!("📊 Completed {i}/{count} swaps");
            }
        }

        Ok(())
    }

    async fn execute_swap(&self, amount_in: U256, path: Vec<Address>) -> Result<()> {
        let expected_out = self
            .router
            .getAmountsOut(amount_in, path.clone())
            .call()
            .await?[1];

        let from_token = if path[0] == *self.usdc.address() {
            &self.usdc
        } else {
            &self.weth
        };

        self.mint_and_approve(from_token, amount_in).await?;
        self.router
            .swapExactTokensForTokens(amount_in, expected_out, path, self.signer)
            .submit()
            .await?;

        Ok(())
    }

    async fn add_initial_liquidity(&self, weth_amount: U256, usdc_amount: U256) -> Result<()> {
        self.mint_and_approve(&self.weth, weth_amount).await?;
        self.mint_and_approve(&self.usdc, usdc_amount).await?;

        self.router
            .addLiquidity(
                *self.weth.address(),
                *self.usdc.address(),
                weth_amount,
                usdc_amount,
            )
            .submit()
            .await?;
        Ok(())
    }

    async fn mint_and_approve(
        &self,
        token: &Erc20::Erc20Instance<P, N>,
        amount: U256,
    ) -> Result<()> {
        token.mint(self.signer, amount).submit().await?;
        token
            .approve(*self.router.address(), amount)
            .submit()
            .await?;
        Ok(())
    }
}
