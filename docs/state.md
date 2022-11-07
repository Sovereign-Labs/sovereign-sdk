# State Model

## Background

One of the primary functions of blockchains is to provide updatable long-term storage which is accessible to programs on-chain.
Importantly, this storage imposes several long-term cost on the network:

- All full nodes need a copy of this data, which significantly slows down initial sync
- Storage proofs become larger (or, in a zk-setting, more expensive to compute)
- Full nodes need to store intermediate hashes of this data, so all costs are amplified by a 32 log(S)

An important design goal for blockchains is to minimize the long-term storage burden. To date, several proposals have been espoused:

- State rent: state is automatically deleted unless it pays rent to the chain
- State expiry: state is deleted if it goes too long without being accessed
- Strong Statelessness: users are required to "bring their own state" when they transact.

Note that we don't consider "weak statelessness" to be a solution to storage growth. Although it reduces the long-term costs on the network
by alleviating the burden on non-consensus nodes, it does nothing to address the burden placed on validators. But BSC has already demonstrated
that very large state sizes can cause problems for validators as well.

### Position

We don't believe that state rent or state expiry (or, more broadly, any scheme that depends on involuntary deletion of state) can address
the problems of storage growth. By nature, top-down forcible deletion carries a risk of contamination - where the deletion of one contract's state interferes with the functioning of another contract that relied on it.

Instead, we believe that blockchains should attempt to price storage so that prices accurately reflect the long term costs to the network,
and leave the allocation of storage slots to market mechanisms. What follow is our attempt to design such a mdel

## Design

We propose two simple models for state storage:

1. Strong statelessness: users are required to bring their own state (meaning, in practice, that RPC providers keep the state for popular
   contracts, and end-users keep theirs. The problem with this model is that no one is responsible for storing account state. )
2. (Recommended) Price storage appropriately based on a model where costs shrink polynomially over time (i.e. slower than and
   exponential like moore's law, but faster than linear)

Since storage costs decrease super-linearly, we propose a model which simply sets a target state growth rate and increases the cost exponentially as that rate is exceeded (a la EIP-1559)

The details of the proposal are as follows:

- Maintain a two-tiered resource usage system, where transactions are charged separately
  for compute (at a rate which adjusts rapidly with demand) and storage (where prices adjust more slowly to target a long-term trend). In other
  words, the relative costs of compute operations to one another should be fixed, while the relative cost of storage floats freely against
  that rate.
- Put a floor on the price of storage growth, so that it can never be too drastically underpriced
- Create incentives for removing state and for creating stateless _contracts_. To help contracts work statelessly, make it easy for apps to
  process transactions in batches.

We assume the existence of a cache containing the original value of the storage slot and
the last value written. The price of storage can be divided into a few quantities:

- `TRIE_UPDATE_COST`: the computational cost of updating an item in the MPT
- `DB_ITEM_COST`: the impact of storing additional db item _forever_ on the operating costs of a full node
- `TRIE_LOAD_COST`: the computational cost of reading an item from the MPT
- `CACHE_HIT_COST`: The computational/
- `CACHE_UPDATE_COST`:

- Each SLOAD operation costs `CACHE_HIT_COST` for a hot access and `TRIE_LOAD_COST` (?) gas for a cold access.
- Each SSTORE operation costs the price of an SLOAD from the same value +

  - If the value == last_written_value unchanged, no additional fee
  - If the original value is nonzero, and the new value is zero, refund up to (`DB_ITEM_COST` / `REFUND_QUOTIENT`) - `TRIE_UPDATE_COST`

    - If the value was null, `TRIE_UPDATE_COST` + `DB_ITEM_COST`

  - Else: `TRIE_UPDATE_COST`
  - If the previous value is non-null and the new value is non-null, `TRIE_UPDATE_COST`
  - If the old value is non-null and the new value is null, `TRIE_UPDATE_COST` - (`DB_ITEM_COST` / `REFUND_QUOTIENT`):
  - This type creates a gas token if `DB_ITEM_COST` / `REFUND_QUOTIENT` > `TRIE_UPDATE_COST`
  - https://eips.ethereum.org/EIPS/eip-2200#specification
  - https://eips.ethereum.org/EIPS/eip-3529

- Each

## Stateless Smart Contracts

To enable stateless smart contracts, you need users to bring merkle proofs of their own state. Unfortunately, this model
only allows a single transaction per-block in the Ethereum execution model, because the first tx to interact with the
stateless contract changes its state root, which invalidates the witnesses of any later transactions. To support this pattern, we
would need one of the following:

- Batch processing at the smart-contract level
- Ephemeral storage.

It's easy to see how batch processing would solve the dilemma. The smart contract would simply verify the witnesses of new transactions
as they arrived, but would defer any state changes until the end of the block. What's harder to see is how the chain would meter gas.
Is it shared evenly across all users who touched a particular contract? etc?

Ephemeral storage is a much better solution. Using this model, contracts could simply process transactions in order, while using
ephemeral storage to allow witness checking.

For example, consider an ERC-20 contract. To implement statelessness, it would need to store a single root hash in _persistent_ storage.
When the first transaction of the block came in (say, a transfer of 5 tokens from Alice to Bob), the contract would do three things:

1. Validate the witness provided by Alice, which proves her and Bob's balances
2. Write the current root hash to ephemeral storage.
3. Write Alice and Bob's new balances to ephemeral storage
4. Update the "balances" root hash.

When the next transaction comes in (say, from Alice -> Charlie), the contract's task is still simple:

1. Check the cache for Alice and Charlie's balances. If either one is in cache, skip verification of the witness for that balance only.
2. Validate any remaining witnesses against the original state root in ephemeral storage
3. Compute new balances and add/update them in ephereral storage
4. Update the balances root hash

We can observe that the smart contract doesn't need any fancy logic to distinguish between these two cases. The entire flow can be unified
like this:

1. If there is no root hash in ephemeral storage, add the save the current root hash.
2. For each balance to be accessed, check the cache. If it doesn't exist, validate the witness against the
   root hash in ephemeral storage, then add that balance to the cache.
3. Make any necessary computations based on the data in cache.
4. Write any changed values to both the cache and the state root in _storage_

It's worth noting that many contracts cannot make use of statelessness (because it's not always possible for Alice to know in advance which
data her transaction will touch, so she can't provide the appropriate witnesses)

We should also consider transaction-length ephemeral storage a la https://eips.ethereum.org/EIPS/eip-1153
