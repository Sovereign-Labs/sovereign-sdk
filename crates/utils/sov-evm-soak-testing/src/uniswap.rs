use alloy::{network::Network, providers::Provider};
use alloy_primitives::{utils::parse_ether, Address, U256};
use anyhow::Result;
use rand::Rng;
use sov_test_utils::{Erc20, Router, Submit};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct SwapMetrics {
    pub total_swaps: usize,
    pub total_time: Duration,
    pub get_amounts_out_time: Duration,
    pub swap_execution_time: Duration,
}

impl SwapMetrics {
    pub fn print_summary(&self) {
        if self.total_swaps == 0 {
            return;
        }

        println!("\n📊 Swap Metrics Summary");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("Total swaps:              {}", self.total_swaps);
        println!("Total time:               {:?}", self.total_time);
        println!(
            "Average per swap:         {:?}",
            self.total_time / self.total_swaps as u32
        );
        println!("Get amounts out (total):  {:?}", self.get_amounts_out_time);
        println!(
            "Get amounts out (avg):    {:?}",
            self.get_amounts_out_time / self.total_swaps as u32
        );
        println!("Swap execution (total):   {:?}", self.swap_execution_time);
        println!(
            "Swap execution (avg):     {:?}",
            self.swap_execution_time / self.total_swaps as u32
        );
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }
}

pub struct UniSoakTest<P, N> {
    weth: Erc20::Erc20Instance<P, N>,
    usdc: Erc20::Erc20Instance<P, N>,
    router: Router::RouterInstance<P, N>,
    signer: Address,
    metrics: SwapMetrics,
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
            metrics: SwapMetrics::default(),
        })
    }

    pub async fn run(mut self, count: usize) -> Result<()> {
        // Larger pool for load testing - 10k WETH : 20k USDC
        println!("💧 Adding liquidity: 10,000 WETH + 20,000 USDC");
        let weth_liquidity = parse_ether("10000")?;
        let usdc_liquidity = parse_ether("20000")?;
        self.add_initial_liquidity(weth_liquidity, usdc_liquidity)
            .await?;

        // Pre-mint large token balances and approve infinite allowance
        println!("🪙 Pre-minting tokens and approving router...");
        let large_amount = parse_ether("1000000000")?; // 1 billion tokens
        self.mint_and_approve(&self.weth, large_amount).await?;
        self.mint_and_approve(&self.usdc, large_amount).await?;

        println!("🔄 Starting load test: {count} random swaps...");
        self.execute_random_swaps(count).await?;

        println!("✅ Load test completed successfully!");
        self.metrics.print_summary();
        Ok(())
    }

    async fn execute_random_swaps(&mut self, count: usize) -> Result<()> {
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

    async fn execute_swap(&mut self, amount_in: U256, path: Vec<Address>) -> Result<()> {
        let swap_total = Instant::now();

        let get_amounts_out = Instant::now();
        let expected_out = self
            .router
            .getAmountsOut(amount_in, path.clone())
            .call()
            .await?[1];
        let get_amounts_out_time = get_amounts_out.elapsed();

        let swap_execution = Instant::now();
        self.router
            .swapExactTokensForTokens(amount_in, expected_out, path, self.signer)
            .submit()
            .await?;
        let swap_execution_time = swap_execution.elapsed();

        let swap_total_time = swap_total.elapsed();

        self.metrics.total_swaps += 1;
        self.metrics.total_time += swap_total_time;
        self.metrics.get_amounts_out_time += get_amounts_out_time;
        self.metrics.swap_execution_time += swap_execution_time;

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
