const { providers, Contract, utils, constants } = require('ethers');
const routerArtifact = require('@uniswap/v2-periphery/build/UniswapV2Router02.json')
const usdtArtifact = require("../artifacts/contracts/Tether.sol/Tether.json")
const usdcArtifact = require("../artifacts/contracts/UsdCoin.sol/UsdCoin.json")

// Copy Addresses
USDT_ADDRESS= '0x5FbDB2315678afecb367f032d93F642f64180aa3'
USDC_ADDRESS= '0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512'
WETH_ADDRESS= '0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0'
FACTORY_ADDRESS= '0xa513E6E4b8f2a923D98304ec87F64353C4D5C853'
PAIR_ADDRESS= '0xD1f1bbbF65CceB5cAf1691a76c17D4E75213B69c'
ROUTER_ADDRESS= '0x8A791620dd6260079BF849Dc5567aDC3F2FdC318'

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
        utils.parseUnits('10', 18),
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