# sov-hyperlane-mailbox

This crate implements a generic `mailbox` for sending and receiving [Hyperlane](https://docs.hyperlane.xyz/) messages.

To use this crate, you need to add four modules to your runtime:
1. `Mailbox` which sends and receives messages
2. `MerkleTreeHook`, a helper module which computes a merkle root of all messages sent from the mailbox
3. `InterchainGasPaymaster`, which handles interchain fees payments to relayers.
4. Any module which implements the `Recipient` trait defined in this crate. This module will be in charge of acting on inbound messages once they've been validated by the mailbox. An example of such module is `Warp`, responsible for intechain token transfers.


## Note for Maintainers
This implementation is highly unusual because Sovereign SDK chains have no concept of runtime-deployed code (i.e. Solana "programs" or Ethereum "smart contracts").
So, in many places where the standard [implementation guide](https://docs.hyperlane.xyz/docs/guides/implementation-guide) suggests using an address to do dynamic dispatch to a handler contract, we've replaced the implementation with
either a generic (as we've done for Mailbox) or an enum (as we've done for InterchainSecurityModules) depending on how much
flexibility it makes sense to give consumers of this module.

Before diving into the code for this crate, it will be very helpful to review the core Hyperlane [protocol docs](https://docs.hyperlane.xyz/docs/protocol/mailbox) and [Alt-VM implementation guide](https://docs.hyperlane.xyz/docs/guides/implementation-guide).
Note that the protocol docs are extremely Ethereum-centric, so don't worry about translating the minutia of the protocol into our domain until you've also read the Alt-VM guide.
