# `sov-evm` module

The sov-evm module provides compatibility with the EVM.

The module `CallMessage` contains `rlp` encoded Ethereum transaction, which is validated & executed immediately after being dispatched from the DA. Once all transactions from the DA slot have been processed, they are grouped into an `Ethereum` block. Users can access information such as receipts, blocks, transactions, and more through standard Ethereum endpoints.


## Genesis

### Accounts

Each EVM account, specified in `data[].address` need to have default credential ID in `accounts.json` genesis and potentially some funds in `bank.json`.

For example, parts of `evm.json`:

```json
{
  "data": [
    {
      "address": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
      "balance": "0xffffffffffffffff",
      "code_hash": "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
      "code": "0x",
      "nonce": 0
    }
  ]
}
```

It needs to have some rollup address generated and placed in `address.json` 
```json
{
  "accounts": [
    {
      "credential_id": "0x000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266",
      "address": "sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3crhxalf"
    }
  ]
}
```

and then 

## Note to developers (hooks.rs)

WARNING: `prevrandao` value is predictable up to `DEFERRED_SLOTS_COUNT` in advance,
Users should follow the same best practice that they would on Ethereum and use future randomness.
See: `<https://eips.ethereum.org/EIPS/eip-4399#tips-for-application-developers>`
