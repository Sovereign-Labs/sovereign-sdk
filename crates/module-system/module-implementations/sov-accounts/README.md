# `sov-accounts` module

The `sov-accounts` module is responsible for managing accounts on the rollup.


### The `sov-accounts` module offers the following functionality:

1. When a sender sends their first message, the `sov-accounts` module will create a new address by deriving it from the sender's credential.
   The module will then add a mapping between the credential id and the address to its state. For all subsequent messages that include the sender's credential,
   the module will retrieve the sender's address from the mapping and pass it along with the original message to an intended module.

1. It is possible to add new credential to a given address using the `CallMessage::InsertCredentialId(..)` message.

1. It is possible to query the `sov-accounts` module using the `get_account` method and get the account corresponding to the given credential id.

