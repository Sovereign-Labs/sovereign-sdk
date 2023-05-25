# This is an example of an `SOV-MODULE`.

It demonstrates the following concepts:
### 1. Module structure:
- `lib.rs` contains `sov_election` module definition and `sov_modules_api::Module` trait implementation for `sov_election`.
- `genesis.rs` contains the module initialization logic.
- `call.rs` contains methods that change module state in response to `CallMessage`.
- `query.rs` contains functions for querying the module state.

### 2. Functionality: 
This module demonstrates the functionality of an election where a group of 'voters' vote for 'candidates' to determine a winner. Please note that this module serves only as an example and should not be used in real scenarios. As an exercise, check how the winner is chosen in the case of a tie between multiple candidates.

The `sov_election` module has a special role called `admin` that is set in `sov_election` genesis method. Only the `admin` can set `candidates` and register `voters`. Once registered, `voter` vote for a chosen `candidate`. After some period of time the `admin` freezes the election and anyone can query who is the winner. The `sov_election` module determines the winner and ensures that the election was fair, for example, by checking that each `voter` voted only once. 

Please, check comments in the `genesis.rs, call.rs & query.rs` for implementation details.