# `sov-revenue-share`

A Sovereign SDK module that automates revenue sharing for your rollup.

To comply with the Sovereign Permissionless Commercial License, any application that generates revenue must share a portion of it if the revenue generating transaction is processed by the preferred sequencer. This module provides the tools to implement this requirement correctly and automatically.

## How It Works

The `sov-revenue-share` module holds funds on behalf of the Sovereign team. When your application logic determines that a fee should be paid, you can direct a percentage of that fee into this module's account.

Key properties:

*   **Default Share:** The revenue share is initially set to 10% (1,000 basis points). This can only be lowered by the Sovereign Admin.
*   **Activation:** Revenue sharing is disabled by default after genesis. The Sovereign Admin must send an `ActivateRevenueShare` transaction to begin collecting fees.
*   **Conditional Logic:** Revenue sharing is only required for transactions processed by the preferred sequencer. The module provides the `is_preferred_sequencer` method to check for this condition.
*   **Supported Assets:** The module can handle any token compatible with the `sov-bank` module.

## Implementation Guide

The `sov-revenue-share` module is included by default in the Sovereign SDK starter template. You only need to add a reference to it in your application module and implement the sharing logic.

### Step 1: Add a Reference to Your Application Module

In the module that handles revenue collection, add a field of type `sov_revenue_share::RevenueShare<S>`.

```rust, ignore
// In your application module (e.g., src/lib.rs)
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct YourApp<S: Spec> {
    #[id]
    pub id: ModuleId,
    // ... other fields
    #[module]
    pub revenue_share: sov_revenue_share::RevenueShare<S>,
}
```

### Step 2: Implement the Sharing Logic

In your module's business logic, use `is_preferred_sequencer` to check if a fee-generating transaction should contribute to the revenue share. This is the standard implementation pattern:

```rust, ignore
// In a method within `impl<S: Spec> YourApp<S>`
pub fn charge_fee(
    &mut self,
    payer: &S::Address,
    total_fee: Amount,
    token_id: TokenId,
    context: &Context<S>,
    state: &mut impl TxState<S>,
) -> anyhow::Result<()> {
    // First, check if the transaction came from the preferred sequencer.
    if self.revenue_share.is_preferred_sequencer(context, state) {
        // If it did, compute and pay the revenue share from the total fee.
        self.revenue_share.compute_and_pay_revenue_share(payer, token_id, total_fee, state)?;
    }
    // ... continue with your fee logic for the remaining amount
    Ok(())
}
```

This pattern ensures that you only share revenue when required by the license. For more complex scenarios, you can use the granular methods `get_revenue_share_percentage_bps()` and `pay_revenue_share()` to build custom logic.

### Runtime Administration

The Sovereign Admin can manage the module through the following call messages:

| Message                           | Purpose                         | Notes                                         |
| --------------------------------- | ------------------------------- | --------------------------------------------- |
| `ActivateRevenueShare`            | Start collecting fees           | Admin‑only                                    |
| `DeactivateRevenueShare`          | Pause collection                | Existing balances stay                        |
| `LowerRevenuePercentage { bps }` | Lower share percentage          | Cannot increase; `bps` ≤ current and ≤ 10 000 |
| `UpdateSovereignAdmin { addr }`   | Transfer admin role             |                                               |
| `WithdrawRewards { token_id }`    | Send accumulated funds to admin | Fails if balance is zero                      |

## License

This crate is distributed under the Sovereign Commercial License and is only available to rollups that have purchased premium features.