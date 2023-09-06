const {
    Contract, ContractFactory, utils, constants,
} = require("ethers")

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


    const FactoryA = new ContractFactory(factoryArtifact.abi, factoryArtifact.bytecode, owner);
    const factoryA = await FactoryA.deploy(owner.address)

    const txA1 = await factoryA.createPair(usdt.address, usdc.address);
    await txA1.wait()

    const pairAddressA = await factoryA.getPair(usdt.address, usdc.address)
    const pairA = new Contract(pairAddressA, pairArtifact.abi, owner)

    const RouterA = new ContractFactory(routerArtifact.abi, routerArtifact.bytecode, owner);
    const routerA = await RouterA.deploy(factoryA.address, weth.address)
    

    const approvalUsdtOwnerA = await usdt.connect(owner).approve(routerA.address, constants.MaxUint256);
    await approvalUsdtOwnerA.wait();
    const approvalUsdcOwnerA = await usdc.connect(owner).approve(routerA.address, constants.MaxUint256);
    await approvalUsdcOwnerA.wait();
    const approvalUsdtTraderA = await usdt.connect(trader).approve(routerA.address, constants.MaxUint256);
    await approvalUsdtTraderA.wait();
    const approvalUsdcTraderA = await usdc.connect(trader).approve(routerA.address, constants.MaxUint256);
    await approvalUsdcTraderA.wait();
    

    const addLiquidityTxA = await routerA.connect(owner).addLiquidity(
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
    addLiquidityTxA.wait();

    let reservesA

    reservesA = await pairA.getReserves()
    console.log('reservesA', reservesA)


    console.log('USDT_ADDRESS=', `'${usdt.address}'`)
    console.log('USDC_ADDRESS=', `'${usdc.address}'`)
    console.log('WETH_ADDRESS=', `'${weth.address}'`)
    console.log('FACTORY_A_ADDRESS=', `'${factoryA.address}'`)
    console.log('PAIR_A_ADDRESS=', `'${pairAddressA}'`)
    console.log('ROUTER_A_ADDRESS=', `'${routerA.address}'`)
}

/*
npx hardhat node
npx hardhat run --network localhost scripts/01_deployContracts.js
*/


main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });