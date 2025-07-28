use indoc::formatdoc;
use serde_json::json;
use sov_hyperlane_integration::EthAddress;
use sov_modules_api::macros::config_value;
use sov_modules_api::{HexHash, HexString};

use super::helpers::{ANVIL_ACCOUNTS, ANVIL_PORT, EVM_DOMAIN, EVM_MAILBOX};

/// Generates a configuration file for the agents with the given rollup port
pub fn agent_config(rollup_port: u16) -> Vec<u8> {
    let config = json!({
        "chains": {
            "sovtest": {
                "chainId": config_value!("CHAIN_ID"),
                "displayName": "SovTest",
                "domainId": config_value!("HYPERLANE_BRIDGE_DOMAIN"),
                "isTestnet": true,
                "name": "sovtest",
                "nativeToken": {
                    "decimals": 18,
                    "name": "SovToken",
                    "symbol": "sov"
                },
                "protocol": "sovereign",
                "rpcUrls": [{
                    "http": format!("HTTP://host.docker.internal:{rollup_port}")
                }],
                // note: here we don't do much based on contract addresses, but some of those may
                // be needed to set to real addresses in a future
                "domainRoutingIsmFactory": "0x0000000000000000000000000000000000000000",
                "interchainAccountIsm": "0x0000000000000000000000000000000000000000",
                "interchainAccountRouter": "0x0000000000000000000000000000000000000000",
                "mailbox": "0x0000000000000000000000000000000000000000",
                "proxyAdmin": "0x0000000000000000000000000000000000000000",
                "staticAggregationHookFactory": "0x0000000000000000000000000000000000000000",
                "staticAggregationIsmFactory": "0x0000000000000000000000000000000000000000",
                "staticMerkleRootMultisigIsmFactory": "0x0000000000000000000000000000000000000000",
                "staticMerkleRootWeightedMultisigIsmFactory": "0x0000000000000000000000000000000000000000",
                "staticMessageIdMultisigIsmFactory": "0x0000000000000000000000000000000000000000",
                "staticMessageIdWeightedMultisigIsmFactory": "0x0000000000000000000000000000000000000000",
                "testRecipient": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "validatorAnnounce": "0x0000000000000000000000000000000000000000",
                "merkleTreeHook": "0x0000000000000000000000000000000000000000",
                "interchainGasPaymaster": "0x0000000000000000000000000000000000000000"
            },
            "ethtest": {
                "chainId": EVM_DOMAIN,
                "displayName": "EthTest",
                "domainId": EVM_DOMAIN,
                "isTestnet": true,
                "name": "ethtest",
                "nativeToken": {
                    "decimals": 18,
                    "name": "Ether",
                    "symbol": "ETH"
                },
                "protocol": "ethereum",
                "rpcUrls": [{
                    "http": format!("HTTP://127.0.0.1:{ANVIL_PORT}")
                }],
                "domainRoutingIsmFactory": "0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9",
                "interchainAccountIsm": "0x9A676e781A523b5d0C0e43731313A708CB607508",
                "interchainAccountRouter": "0x68B1D87F95878fE05B998F19b66F4baba5De1aed",
                "mailbox": EVM_MAILBOX.to_string(),
                "merkleTreeHook": "0xB7f8BC63BbcaD18155201308C8f3540b07f84F5e",
                "proxyAdmin": "0xa513E6E4b8f2a923D98304ec87F64353C4D5C853",
                "staticAggregationHookFactory": "0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9",
                "staticAggregationIsmFactory": "0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0",
                "staticMerkleRootMultisigIsmFactory": "0x5FbDB2315678afecb367f032d93F642f64180aa3",
                "staticMerkleRootWeightedMultisigIsmFactory": "0x5FC8d32690cc91D4c39d9d3abcBD16989F875707",
                "staticMessageIdMultisigIsmFactory": "0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512",
                "staticMessageIdWeightedMultisigIsmFactory": "0x0165878A594ca255338adfa4d48449f69242Eb8F",
                "testRecipient": "0xc6e7DF5E7b4f2A278906862b61205850344D4e7d",
                "validatorAnnounce": "0x3Aa5ebB10DC797CAC828524e59A333d0A371443c",
                "interchainGasPaymaster": "0x0000000000000000000000000000000000000000",
                "index": {
                    "from": 9
                }
            }
        },
        "defaultRpcConsensusType": "fallback"
    });

    serde_json::to_vec(&config).unwrap()
}

/// Core config used by hyperlane-cli
///
/// Result of setting up ethtest/metadata.yaml, sovtest/metadata.yaml and running
/// `hyperlane core init --advanced` and selecting defaults for everything except:
///  ism: testIsm
///  default hook: merkleTreeHook
///  required hook: protocolFee
pub fn core_config(owner: EthAddress) -> String {
    formatdoc! {"
        defaultHook:
          type: merkleTreeHook
        defaultIsm:
          type: testIsm
        owner: \"{owner}\"
        proxyAdmin:
          owner: \"{owner}\"
        requiredHook:
          beneficiary: \"{owner}\"
          maxProtocolFee: \"0\"
          owner: \"{owner}\"
          protocolFee: \"0\"
          type: protocolFee
    "}
}

/// Configuration of sovtest chain in hyperlane
pub fn sovtest_metadata(rollup_port: u16) -> String {
    let chain = config_value!("CHAIN_ID");
    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    formatdoc! {"
        chainId: {chain}
        displayName: SovTest
        domainId: {domain}
        isTestnet: true
        name: sovtest
        nativeToken:
          decimals: 18
          name: SovToken
          symbol: sov
        protocol: ethereum
        rpcUrls:
          - http: HTTP://host.docker.internal:{rollup_port}
    "}
}

/// Configuration of sovtest smart contract addressess in hyperlane
///
/// Sov implementation uses modules thus we use only dummy addresses here.
pub fn sovtest_addresses(sov_test_recipient: HexHash) -> String {
    formatdoc! {"
        domainRoutingIsmFactory: \"0x0000000000000000000000000000000000000000\"
        interchainAccountIsm: \"0x0000000000000000000000000000000000000000\"
        interchainAccountRouter: \"0x0000000000000000000000000000000000000000\"
        mailbox: \"0x0000000000000000000000000000000000000000\"
        proxyAdmin: \"0x0000000000000000000000000000000000000000\"
        staticAggregationHookFactory: \"0x0000000000000000000000000000000000000000\"
        staticAggregationIsmFactory: \"0x0000000000000000000000000000000000000000\"
        staticMerkleRootMultisigIsmFactory: \"0x0000000000000000000000000000000000000000\"
        staticMerkleRootWeightedMultisigIsmFactory: \"0x0000000000000000000000000000000000000000\"
        staticMessageIdMultisigIsmFactory\": \"0x0000000000000000000000000000000000000000\"
        staticMessageIdWeightedMultisigIsmFactory: \"0x0000000000000000000000000000000000000000\"
        testRecipient: \"{sov_test_recipient}\"
        validatorAnnounce: \"0x0000000000000000000000000000000000000000\"
        merkleTreeHook: \"0x0000000000000000000000000000000000000000\"
        interchainGasPaymaster: \"0x0000000000000000000000000000000000000000\"
    "}
}

/// Configuration of ethtest chain in hyperlane
pub fn ethtest_metadata() -> String {
    formatdoc! {"
        chainId: {EVM_DOMAIN}
        displayName: EthTest
        domainId: {EVM_DOMAIN}
        isTestnet: true
        name: ethtest
        nativeToken:
          decimals: 18
          name: Ether
          symbol: ETH
        protocol: ethereum
        rpcUrls:
          - http: HTTP://127.0.0.1:{ANVIL_PORT}
    "}
}

/// Configuration for deploying warp route on evm counterparty
///
/// Optionally accepts a route id on sovtest chain, to enroll
/// a router on ethtest.
///
/// Examples of warp route configs can be found here: <https://docs.hyperlane.xyz/docs/guides/extending-warp-route>
pub fn warp_route_config(sov_route_id: HexHash) -> String {
    let owner = ANVIL_ACCOUNTS[0].0;
    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    formatdoc! {"
        ethtest:
          isNft: false
          type: native
          name: \"EthNativeToken\"
          symbol: \"nativeETH\"
          decimals: 18
          totalSupply: 0
          owner: \"{owner}\"
          interchainSecurityModule: \"0x0000000000000000000000000000000000000000\"
          remoteRouters:
            \"{domain}\":
              \"address\": \"{sov_route_id}\"
    "}
}

/// A configuration of deployed route between ethtest and sovtest.
///
/// This configuration file should be generated by `hyperlane warp deploy|apply`, however
/// `hyperlane-cli` is currently unable to generate the sov side, so we need to put it manually.
///
/// Examples of this file can be found here:
/// <https://docs.hyperlane.xyz/docs/protocol/warp-routes/warp-routes-custom-gas-fast-native#4-deploy-a-native-warp-route>
pub fn warp_route_deployment(
    ethtest_route: HexHash,
    sovtest_route: HexHash,
    sovtest_decimals: u8,
) -> String {
    let eth_addr = HexString(&ethtest_route.0[12..]);
    formatdoc! {"
        tokens:
          - addressOrDenom: \"{eth_addr}\"
            chainName: ethtest
            decimals: 18
            name: EthNativeToken
            standard: EvmHypNative
            symbol: nativeETH
            connections:
              - type: hyperlane
                token: ethereum|sovtest|{sovtest_route}
          - addressOrDenom: \"{sovtest_route}\"
            chainName: sovtest
            decimals: {sovtest_decimals}
            name: EthNativeToken
            standard: ERC20
            symbol: nativeETH
    "}
}
