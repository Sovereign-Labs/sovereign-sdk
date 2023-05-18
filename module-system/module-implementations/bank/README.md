# The Bank module.

The `Bank` module is responsible managing `token` creation and `transfers`.

### The Bank module offers the following functionality:

Calls:
1. `CallMessage::CreateToken` message creates a new `token` with initial balance for the minter with. Conceptually a token is a mapping from users addresses to balances. Each token has a name and a unique address created automatically by the bank module during the creation phase.

1. `CallMessage::Transfer` message transfers tokens between two accounts. The sender has to specify a beneficiary and the token address.

1. `CallMessage::Burn` message burns the specified amount of tokens.

Queries::
1. `QueryMessage::GetBalance` query returns balance of a specific token for given user.
1. `QueryMessage::GetTotalSupply` query returns total supply of a specific token.



