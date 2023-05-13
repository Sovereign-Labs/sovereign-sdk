# The Accounts module.
The `Accounts` module is responsible for managing accounts on the rollup. 

Account is represented by an `Address` and a `Nonce`.

## It offers the following functionality:

1. When a sender sends their first message, the `Accounts` module will create a new address by deriving it from the sender's public key.
The module will then add a mapping between the public key and the address to its state. For all subsequent messages that include the sender's public key, the module will retrieve the sender's address from the mapping and pass it along with the original message to the intended module.

1. It is possible to update the public key associated with a given address using the `CallMessage::UpdatePublicKey(..)` message. To do so, the sender must prove that they possess the private key that corresponds to the new public key.

1. Account nonce is increased on every processed message, the nonce guards against double spending attack and introduces a tx ordering.. 

1. It is possible to query the `Accounts` module with `QueryMessage::GetAccount(..)` message ang get account corresponding to the given public key.

## The Accounts module makes the following guarantees:

1. The mapping between public keys and addresses is one-to-one (bijective).

1. At some point in time, the sender has provided proof that they possessed the private key corresponding to the public key associated with the address.

1. Account nonce is increased on every processed message by 1.



