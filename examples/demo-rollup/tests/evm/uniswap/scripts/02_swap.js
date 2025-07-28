require("dotenv").config();

const { ethers } = require("hardhat");
const routerArtifact = require("@uniswap/v2-periphery/build/UniswapV2Router02.json");
const usdtArtifact = require("../artifacts/contracts/Tether.sol/Tether.json");
const usdcArtifact = require("../artifacts/contracts/UsdCoin.sol/UsdCoin.json");

USDT_ADDRESS = process.env.USDT_ADDRESS;
USDC_ADDRESS = process.env.USDC_ADDRESS;
WETH_ADDRESS = process.env.WETH_ADDRESS;
FACTORY_ADDRESS = process.env.FACTORY_ADDRESS;
PAIR_ADDRESS = process.env.PAIR_ADDRESS;
ROUTER_ADDRESS = process.env.ROUTER_ADDRESS;

const provider = ethers.provider;

const router = new ethers.Contract(ROUTER_ADDRESS, routerArtifact.abi, provider);

const usdt = new ethers.Contract(USDT_ADDRESS, usdtArtifact.abi, provider);

const usdc = new ethers.Contract(USDC_ADDRESS, usdcArtifact.abi, provider);

const logBalance = async (signerObj) => {
  let ethBalance;
  let usdtBalance;
  let usdcBalance;
  let balances;
  ethBalance = await signerObj.getBalance();
  usdtBalance = await usdt.balanceOf(signerObj.address);
  usdcBalance = await usdc.balanceOf(signerObj.address);
  balances = {
    ethBalance: ethBalance,
    usdtBalance: usdtBalance,
    usdcBalance: usdcBalance
  };
  console.log(`balances of ${signerObj.address}`, balances);
};

const main = async () => {
  const [owner, trader] = await ethers.getSigners();
  if (!owner || !owner.address || !trader || !trader.address) {
    throw new Error("Could not get owner and trader addresses");
  }
  if (!ROUTER_ADDRESS) {
    throw new Error("Uniswap router address is not detected.");
  }

  console.log(
    `Starting with owner=${owner.address} and trader=${trader.address} uniswap router=${ROUTER_ADDRESS}`
  );

  await logBalance(owner);
  await logBalance(trader);

  await logBalance(owner, usdt, usdc);
  await logBalance(trader, usdt, usdc);

  console.log("Swapping USDT/USDC");
  const tx = await router
    .connect(trader)
    .swapExactTokensForTokens(
      ethers.utils.parseUnits("2", 18),
      ethers.utils.parseUnits("1", 18),
      [USDT_ADDRESS, USDC_ADDRESS],
      trader.address,
      Math.floor(Date.now() / 1000) + 60 * 10,
      {
        gasLimit: 1000000
      }
    );

  await tx.wait();
  await logBalance(owner, usdt, usdc);
  await logBalance(trader, usdt, usdc);
};

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
