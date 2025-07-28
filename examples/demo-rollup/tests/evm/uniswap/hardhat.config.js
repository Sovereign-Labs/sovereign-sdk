require("@nomiclabs/hardhat-ethers");

/** @type import('hardhat/config').HardhatUserConfig */
module.exports = {
  solidity: {
    version: "0.8.17",
    settings: {
      optimizer: {
        enabled: true,
        runs: 5000,
        details: { yul: false },
      },
    },
  },
  networks: {
    sovereign: {
      url: "http://127.0.0.1:12345",
      chainId: 1,
      accounts: [
        // For demo rollup, please check corresponding addresses are present in
        // mock da: ../../../../test-data/genesis/demo/mock/evm.json
        // celestia da: ../../../../test-data/genesis/demo/celestia/evm.json
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "0x96eeea10d406ba7d4e74f7bb9e71b6378165162e4e42fd31c937f7728bbaa7b2",
        "0x90cb5be9e2c125d84af44f19a4e6e36af359bd47b41577aedbe8aa24313bbd40",
      ],
    },
  },
};
