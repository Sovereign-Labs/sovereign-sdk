# `sov-paymaster`

The `Paymaster` module allows third parties to buy gas on behalf of a user. 

Payers are configured per-sequencer, and the rollup falls back to having the user pay their own fees
if the configured payer will not.
