# The Bank module.

The `Bank` module is responsible for managing tokens on the rollup.

### The Bank module offers the following functionality:

Calls:

1. The `CallMessage::CreateToken` message creates a new `token` with  an initial balance allocated to the minter. Conceptually a token is a mapping from users addresses to balances. Each token has a name and a unique address created automatically by the `bank` module during the creation phase.

1. The `CallMessage::Transfer` message facilitates the transfer of tokens between two accounts. To initiate the transfer, the sender must provide the beneficiary's account, the amount of tokens to be transferred, and the token address. It is important to note that the sender's account balance must be greater than the amount being transferred.

1. The `CallMessage::Burn` message burns the specified amount of tokens.

Queries:
1. The `QueryMessage::GetBalance` query retrieves the balance of a specific token for a given user.

1. The `QueryMessage::GetTotalSupply` query retrieves total supply of a specific token.
