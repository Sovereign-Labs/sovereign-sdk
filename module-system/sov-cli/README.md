# CLI Wallet

This package defines a CLI wallet to be used with the Sovereign SDK

## Storage
By default, this wallet persists data in a directory called `.sov_cli_wallet`, under your home directory. Home is defined as follows:
- Linux:   `/home/alice/`
- Windows: `C:\Users\Alice\AppData\Roaming`
- macOS:   `/Users/Alice/Library/Application Support`

To override this behavior, set the `SOV_WALLET_DIR` environment variable to the desired directory. Note that this directory is treated as a complete path, so the `.sov_cli_wallet` suffix is not automatically appended.
