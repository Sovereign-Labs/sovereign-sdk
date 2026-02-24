use indoc::{formatdoc, indoc};
use serde_json::json;
use sov_hyperlane_integration::EthAddress;
use sov_modules_api::macros::config_value;

use super::helpers::{
    EVM_CHAIN_ID, EVM_DOMAIN, EVM_MAILBOX, EVM_MERKLE_TREE_HOOK, EVM_TEST_RECIPIENT,
    RELAYER_ACCOUNT,
};

const EVM_SNAPSHOT_BLOCK: u32 = 19;

/// Generates a configuration file for the agents with the given rollup port
pub fn agent_config(rollup_port: u16, anvil_port: u16, host_address: &str) -> Vec<u8> {
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
                    "http": format!("http://{}:{}", host_address, rollup_port)
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
                "chainId": EVM_CHAIN_ID,
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
                    "http": format!("http://{}:{}", host_address, anvil_port)
                }],
                "domainRoutingIsmFactory": "0xe1Aa25618fA0c7A1CFDab5d6B456af611873b629",
                "interchainAccountIsm": "0x0000000000000000000000000000000000000000",
                "interchainAccountRouter": "0x2a264F26859166C5BF3868A54593eE716AeBC848",
                "mailbox": EVM_MAILBOX.to_string(),
                "merkleTreeHook": EVM_MERKLE_TREE_HOOK.to_string(),
                "proxyAdmin": "0xeD1DB453C3156Ff3155a97AD217b3087D5Dc5f6E",
                "staticAggregationHookFactory": "0x8ce361602B935680E8DeC218b820ff5056BeB7af",
                "staticAggregationIsmFactory": "0xb19b36b1456E65E3A6D514D3F715f204BD59f431",
                "staticMerkleRootMultisigIsmFactory": "0x700b6A60ce7EaaEA56F065753d8dcB9653dbAD35",
                "staticMerkleRootWeightedMultisigIsmFactory": "0xe1DA8919f262Ee86f9BE05059C9280142CF23f48",
                "staticMessageIdMultisigIsmFactory": "0xA15BB66138824a1c7167f5E85b957d04Dd34E468",
                "staticMessageIdWeightedMultisigIsmFactory": "0x0C8E79F3534B00D9a3D4a856B665Bf4eBC22f2ba",
                "testRecipient": EVM_TEST_RECIPIENT.to_string(),
                "validatorAnnounce": "0xd04fF4A75Edd737A73E92b2F2274Cb887d96E110",
                "interchainGasPaymaster": "0x0000000000000000000000000000000000000000",
                // Snapshot only retains the latest block, so don't subtract a reorg window.
                "blocks": {
                    "confirmations": 1,
                    "estimateBlockTime": 1,
                    "reorgPeriod": 0
                },
                "index": {
                    "from": EVM_SNAPSHOT_BLOCK
                }
            }
        },
        "defaultRpcConsensusType": "fallback"
    });

    tracing::info!(?config, "Agent config");
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
pub fn sovtest_metadata(rollup_port: u16, host_address: &str) -> String {
    let chain = config_value!("CHAIN_ID");
    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    formatdoc! {"
        chainId: sovtest-{chain}
        displayName: SovTest
        domainId: {domain}
        isTestnet: true
        name: sovtest
        nativeToken:
          decimals: 8
          name: SovToken
          symbol: sov
        protocol: sovereign
        rpcUrls:
          - http: http://{host_address}:{rollup_port}
    "}
}

/// Configuration of sovtest smart contract addresses in hyperlane
///
/// Sov implementation uses modules thus we use only dummy addresses here.
pub fn sovtest_addresses() -> &'static str {
    indoc! {"
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
        testRecipient: \"0x0000000000000000000000000000000000000000\"
        validatorAnnounce: \"0x0000000000000000000000000000000000000000\"
        merkleTreeHook: \"0x0000000000000000000000000000000000000000\"
        interchainGasPaymaster: \"0x0000000000000000000000000000000000000000\"
    "}
}

/// Configuration of ethtest smart contract addresses in hyperlane.
pub fn ethtest_addresses() -> String {
    formatdoc! {"
        domainRoutingIsmFactory: \"0xe1Aa25618fA0c7A1CFDab5d6B456af611873b629\"
        interchainAccountIsm: \"0x0000000000000000000000000000000000000000\"
        interchainAccountRouter: \"0x2a264F26859166C5BF3868A54593eE716AeBC848\"
        mailbox: \"{EVM_MAILBOX}\"
        merkleTreeHook: \"{EVM_MERKLE_TREE_HOOK}\"
        proxyAdmin: \"0xeD1DB453C3156Ff3155a97AD217b3087D5Dc5f6E\"
        staticAggregationHookFactory: \"0x8ce361602B935680E8DeC218b820ff5056BeB7af\"
        staticAggregationIsmFactory: \"0xb19b36b1456E65E3A6D514D3F715f204BD59f431\"
        staticMerkleRootMultisigIsmFactory: \"0x700b6A60ce7EaaEA56F065753d8dcB9653dbAD35\"
        staticMerkleRootWeightedMultisigIsmFactory: \"0xe1DA8919f262Ee86f9BE05059C9280142CF23f48\"
        staticMessageIdMultisigIsmFactory: \"0xA15BB66138824a1c7167f5E85b957d04Dd34E468\"
        staticMessageIdWeightedMultisigIsmFactory: \"0x0C8E79F3534B00D9a3D4a856B665Bf4eBC22f2ba\"
        testRecipient: \"{EVM_TEST_RECIPIENT}\"
        validatorAnnounce: \"0xd04fF4A75Edd737A73E92b2F2274Cb887d96E110\"
        interchainGasPaymaster: \"0x0000000000000000000000000000000000000000\"
    "}
}

/// Configuration of ethtest chain in hyperlane
pub fn ethtest_metadata(anvil_host: &str, anvil_port: u16) -> String {
    formatdoc! {"
        chainId: {EVM_CHAIN_ID}
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
          - http: http://{anvil_host}:{anvil_port}
    "}
}

/// Configuration for deploying the warp route on evm counterparty.
/// sovtest chain is updated in a separate call.
///
/// Examples of warp route configs can be found here: <https://docs.hyperlane.xyz/docs/guides/extending-warp-route>
pub fn warp_route_config() -> String {
    let owner = RELAYER_ACCOUNT.0;
    formatdoc! {"
        ethtest:
          type: native
          name: \"EthNativeToken\"
          symbol: \"nativeETH\"
          decimals: 18
          owner: \"{owner}\"
          interchainSecurityModule: \"0x0000000000000000000000000000000000000000\"\
    "}
}
