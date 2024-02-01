const {
    Contract, ContractFactory, utils, constants,
} = require("ethers")

const fs = require('fs');
const { promisify } = require('util');

const WETH9 = require("../WETH9.json")

const factoryArtifact = require('@uniswap/v2-core/build/UniswapV2Factory.json')
const routerArtifact = require('@uniswap/v2-periphery/build/UniswapV2Router02.json')
const pairArtifact = require('@uniswap/v2-periphery/build/IUniswapV2Pair.json')


async function main() {
    const [owner, trader] = await ethers.getSigners()

    const Usdt = await ethers.getContractFactory('Tether', owner);
    const usdt = await Usdt.deploy();
    const Usdc = await ethers.getContractFactory('UsdCoin', owner);
    const usdc = await Usdc.deploy();
    const Weth = new ContractFactory(WETH9.abi, WETH9.bytecode, owner);
    const weth = await Weth.deploy();


    const mintAmount = utils.parseEther('100000')
    await usdt.connect(owner).mint(owner.address, mintAmount)
    await usdc.connect(owner).mint(owner.address, mintAmount)
    await usdt.connect(owner).mint(trader.address, mintAmount)
    await usdc.connect(owner).mint(trader.address, mintAmount)


    const Factory = new ContractFactory(factoryArtifact.abi, factoryArtifact.bytecode, owner);
    const factory = await Factory.deploy(owner.address)

    const tx = await factory.createPair(usdt.address, usdc.address);
    await tx.wait()

    const pairAddress = await factory.getPair(usdt.address, usdc.address)
    const pair = new Contract(pairAddress, pairArtifact.abi, owner)

    const Router = new ContractFactory(routerArtifact.abi, routerArtifact.bytecode, owner);
    const router = await Router.deploy(factory.address, weth.address)
    

    const approvalUsdtOwnerA = await usdt.connect(owner).approve(router.address, constants.MaxUint256);
    await approvalUsdtOwnerA.wait();
    const approvalUsdcOwnerA = await usdc.connect(owner).approve(router.address, constants.MaxUint256);
    await approvalUsdcOwnerA.wait();
    const approvalUsdtTraderA = await usdt.connect(trader).approve(router.address, constants.MaxUint256);
    await approvalUsdtTraderA.wait();
    const approvalUsdcTraderA = await usdc.connect(trader).approve(router.address, constants.MaxUint256);
    await approvalUsdcTraderA.wait();
    

    const addLiquidityTx = await router.connect(owner).addLiquidity(
        usdt.address,
        usdc.address,
        utils.parseEther('100'),
        utils.parseEther('100'),
        0,
        0,
        owner.address,
        Math.floor(Date.now() / 1000 + (10 * 60)),
        { gasLimit: utils.hexlify(1_000_000)}
    );
    addLiquidityTx.wait();

    let reserves

    reserves = await pair.getReserves()
    console.log('reservesA', reserves)


    let addresses = [
      `USDT_ADDRESS=${usdt.address}`,
      `USDC_ADDRESS=${usdc.address}`,
      `WETH_ADDRESS=${weth.address}`,
      `FACTORY_ADDRESS=${factory.address}`,
      `PAIR_ADDRESS=${pairAddress}`,
      `ROUTER_ADDRESS=${router.address}`,
    ]


    
    console.log('addresses:', addresses)

    const data = addresses.join('\n')
    const writeFile = promisify(fs.writeFile);
    const filePath = '.env';
    
    
    return writeFile(filePath, data)
        .then(() => {
          console.log('Addresses recorded.');
        })
        .catch((error) => {
          console.error('Error logging addresses:', error);
          throw error;
        });
    
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });