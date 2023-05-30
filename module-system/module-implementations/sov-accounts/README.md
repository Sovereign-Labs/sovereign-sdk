# `sov-accounts` module

The `sov-accounts` module is responsible for managing accounts on the rollup.

Account is represented by an `Address` and a `Nonce`.

## Warning

The accounts module implements `TxHooks` which must be wired into your state transition function! Be sure that your `Runtime` implementation for `TxHooks` delegates to the `sov-accounts.` 

### The `sov-accounts` module offers the following functionality:

1. When a sender sends their first message, the `sov-accounts` module will create a new address by deriving it from the sender's public key.
   The module will then add a mapping between the public key and the address to its state. For all subsequent messages that include the sender's public key,
   the module will retrieve the sender's address from the mapping and pass it along with the original message to an intended module.

1. It is possible to update the public key associated with a given address using the `CallMessage::UpdatePublicKey(..)` message.
   To do so, the sender must prove that they possess the private key that corresponds to the new public key.

1. Each processed message increases the account nonce. This serves to protect against double-spending attacks and ensures proper transaction ordering.

1. It is possible to query the `sov-accounts` module using the `get_account` method and get the account corresponding to the given public key.

### The `sov-accounts` module makes the following guarantees:

1. At some point in time, the sender has provided proof that they possessed the private key corresponding to the public key associated with the address.

1. The account nonce is increased on every processed message by 1.
