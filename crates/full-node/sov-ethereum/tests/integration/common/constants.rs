use sov_sequencer::SeqConfigExtension;

pub const SENDER_PRIV_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub const SECONDARY_SENDER_PRIV_KEY: &str =
    "0x96eeea10d406ba7d4e74f7bb9e71b6378165162e4e42fd31c937f7728bbaa7b2";
/// Hardhat #1: 0x70997970C51812dc3A010C7d01b50e0d17dc79C8
/// Not in any genesis -> starts with zero EVM balance, not covered by selective paymaster.
pub const AFFORDABILITY_SIGNER_PRIV_KEY: &str =
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
/// Hardhat #4: 0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65
/// Not in any genesis -> starts with zero EVM balance, covered by selective paymaster.
pub const PAYMASTER_SIGNER_PRIV_KEY: &str =
    "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a";

pub const EVM_EXTENSION: SeqConfigExtension = SeqConfigExtension {
    max_log_limit: 20000,
    response_size_limit: (1024 * 1024) - (1024 * 30),
};

pub const MAX_FEE_PER_GAS: u128 = 1_000_000_000;
pub const HIGH_MAX_FEE_PER_GAS: u128 = 1_000_000_000_000;
pub const PAYER_SOV_BANK_BALANCE: u128 = 5_000_000_000_000_000;
pub const HIGH_PRIORITY_FEE_PER_GAS: u128 = 1;
pub const MAX_POLL_ATTEMPTS: usize = 100;
pub const POLL_INTERVAL_MS: u64 = 25;

pub const INSUFFICIENT_FUNDS_ERROR: &str = "insufficient funds for gas * price + value";
pub const FEE_CAP_TOO_LOW_ERROR: &str = "max fee per gas less than block base fee";

/// Default per-account balance for EVM addresses pre-funded in test genesis
/// (mirrors `examples/test-data/genesis/integration-tests/bank.json`).
pub const DEFAULT_EVM_BALANCE: u128 = 100_000_000_000_000_000;

/// EVM addresses that are pre-funded in the default test genesis.
/// Matches the EVM accounts in `bank.json` (hardhat-style test addresses).
pub const DEFAULT_FUNDED_EVM_ACCOUNTS: &[&str] = &[
    "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
    "0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71",
    "0x4Fa6c577eE74B4F3C5309Af1b6313dd6D525e694",
    "0xa80749aD39A047603cbc5D0f46a03A1c6B2Db6c9",
];
