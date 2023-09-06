require('dotenv').config()
const fs = require('fs');
const { promisify } = require('util');

const { providers, Contract, utils, constants } = require('ethers');
const routerArtifact = require('@uniswap/v2-periphery/build/UniswapV2Router02.json')
const usdtArtifact = require("../artifacts/contracts/Tether.sol/Tether.json")
const usdcArtifact = require("../artifacts/contracts/UsdCoin.sol/UsdCoin.json")

USDT_ADDRESS = process.env.USDT_ADDRESS
USDC_ADDRESS = process.env.USDC_ADDRESS
WETH_ADDRESS = process.env.WETH_ADDRESS
FACTORY_ADDRESS = process.env.FACTORY_ADDRESS
PAIR_ADDRESS = process.env.PAIR_ADDRESS
ROUTER_ADDRESS = process.env.ROUTER_ADDRESS



const provider = new providers.JsonRpcProvider('http://127.0.0.1:8545/')

const router = new Contract(
    ROUTER_ADDRESS,
    routerArtifact.abi,
    provider
)

const usdt = new Contract(
    USDT_ADDRESS,
    usdtArtifact.abi,
    provider
)

const usdc = new Contract(
    USDC_ADDRESS,
    usdcArtifact.abi,
    provider
)

const logBalance = async (signerObj) => {
    let ethBalance
    let usdtBalance
    let usdcBalance
    let balances
    ethBalance = await signerObj.getBalance()
    usdtBalance = await usdt.balanceOf(signerObj.address)
    usdcBalance = await usdc.balanceOf(signerObj.address)
    balances = {
        ethBalance: ethBalance,
        usdtBalance: usdtBalance,
        usdcBalance: usdcBalance,
    }
    console.log('balances', balances)

}

const main = async () => {
    const [owner, trader] = await ethers.getSigners()

    await logBalance(trader)

    const tx = await router.connect(trader).swapExactTokensForTokens(
        utils.parseUnits('2', 18),
        utils.parseUnits('1', 18),
        [USDT_ADDRESS, USDC_ADDRESS],
        trader.address,
        Math.floor(Date.now() / 1000) + (60 * 10),
        {
            gasLimit: 1000000,
        }
    )

    await tx.wait()
    await logBalance(trader)
}


main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error(error);
        process.exit(1);
    });