const { Contract, ContractFactory, utils, constants } = require("ethers");

const fs = require("fs");
const { promisify } = require("util");

const WETH9 = require("../WETH9.json");

const factoryArtifact = require("@uniswap/v2-core/build/UniswapV2Factory.json");
const routerArtifact = require("@uniswap/v2-periphery/build/UniswapV2Router02.json");
const pairArtifact = require("@uniswap/v2-periphery/build/IUniswapV2Pair.json");

async function main() {
  console.log("Starting Uniswap deployment script...");
  const [owner, trader] = await ethers.getSigners();
  // assert that owner.address and trader.address are not undefined
  if (!owner || !owner.address || !trader || !trader.address) {
    throw new Error("Could not get owner and trader addresses");
  }

  console.log(
    `Starting with owner=${owner.address} and trader=${trader.address}`,
  );

  console.log("Deploying USDT...");
  const Usdt = await ethers.getContractFactory("Tether", owner);
  const usdt = await Usdt.deploy();
  console.log("USDT deployed at:", usdt.address);
  console.log("Deploying USDC...");
  const Usdc = await ethers.getContractFactory("UsdCoin", owner);
  const usdc = await Usdc.deploy();
  console.log("USDC deployed at:", usdc.address);
  console.log("Deploying WETH...");
  const Weth = new ContractFactory(WETH9.abi, WETH9.bytecode, owner);
  const weth = await Weth.deploy();
  console.log("WETH deployed at:", weth.address);

  console.log("Minting some tokens...");
  const mintAmount = utils.parseEther("100000");
  await usdt.connect(owner).mint(owner.address, mintAmount);
  console.log("Minted USDT for owner");
  await usdc.connect(owner).mint(owner.address, mintAmount);
  console.log("Minted USDC for owner");
  await usdt.connect(owner).mint(trader.address, mintAmount);
  console.log("Minted USDT for trader");
  await usdc.connect(owner).mint(trader.address, mintAmount);
  console.log("Minted USDC for trader");
  console.log("All tokens have been minted");

  console.log("Deploying Uniswap factory...");
  const Factory = new ContractFactory(
    factoryArtifact.abi,
    factoryArtifact.bytecode,
    owner,
  );
  const factory = await Factory.deploy(owner.address);
  console.log("Factory deployed at:", factory.address);

  console.log("Creating USDT/USDC pair...");
  const tx = await factory.createPair(usdt.address, usdc.address);
  await tx.wait();
  console.log("USDT/USDC pair created");

  console.log("Deploying Uniswap router...");
  const Router = new ContractFactory(
    routerArtifact.abi,
    routerArtifact.bytecode,
    owner,
  );
  const router = await Router.deploy(factory.address, weth.address);
  console.log("Router deployed at:", router.address);

  const approvalUsdtOwnerA = await usdt
    .connect(owner)
    .approve(router.address, constants.MaxUint256);
  await approvalUsdtOwnerA.wait();
  const approvalUsdcOwnerA = await usdc
    .connect(owner)
    .approve(router.address, constants.MaxUint256);
  await approvalUsdcOwnerA.wait();
  const approvalUsdtTraderA = await usdt
    .connect(trader)
    .approve(router.address, constants.MaxUint256);
  await approvalUsdtTraderA.wait();
  const approvalUsdcTraderA = await usdc
    .connect(trader)
    .approve(router.address, constants.MaxUint256);
  await approvalUsdcTraderA.wait();

  console.log("Adding liquidity...");
  const addLiquidityTx = await router
    .connect(owner)
    .addLiquidity(
      usdt.address,
      usdc.address,
      utils.parseEther("100"),
      utils.parseEther("100"),
      0,
      0,
      owner.address,
      Math.floor(Date.now() / 1000 + 10 * 60),
      { gasLimit: utils.hexlify(1_000_000) },
    );
  addLiquidityTx.wait();
  console.log("Liquidity added");

  const usdtUsdcPairAddress = await factory.getPair(usdt.address, usdc.address);
  const usdtUsdcPair = new Contract(
    usdtUsdcPairAddress,
    pairArtifact.abi,
    owner,
  );
  let reserves = await usdtUsdcPair.getReserves();
  console.log("reserves USDT/USDC", reserves);

  let addresses = [
    `USDT_ADDRESS=${usdt.address}`,
    `USDC_ADDRESS=${usdc.address}`,
    `WETH_ADDRESS=${weth.address}`,
    `FACTORY_ADDRESS=${factory.address}`,
    `PAIR_ADDRESS=${usdtUsdcPairAddress}`,
    `ROUTER_ADDRESS=${router.address}`,
  ];

  console.log("addresses:", addresses);

  const data = addresses.join("\n");
  const writeFile = promisify(fs.writeFile);
  const filePath = ".env";

  return writeFile(filePath, data)
    .then(() => {
      console.log(`Addresses has been recorded to ${filePath}`);
    })
    .catch((error) => {
      console.error("Error logging addresses:", error);
      throw error;
    });
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
