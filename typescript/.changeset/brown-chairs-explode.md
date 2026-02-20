---
"@sovereign-sdk/types": minor
"@sovereign-sdk/web3": minor
---

Add support for rollup height-based uniqueness. A transaction with a uniqueness value of `{ height: N }` will be valid from height N for a short time (depending on rollup configuration), as long as a transaction with the same value and same hash has not already been submitted. To use this, override the standard rollup in one of two ways:
 * Create the rollup with an override. This will configure all transactions submitted through it to use the height mechanism by default; this can be overridden on a case by case basis.
 ```typescript
 import { createStandardRollup, heightUniquenessBuilderOverride } from "@sovereign-sdk/web3";
 const rollup = await createStandardRollup(undefined, heightUniquenessBuilderOverride());
 ```
 * Provide an explicit one-off override for the `uniqueness` when using `rollup.call()` or `rollup.prepareCall()`:
 ```typescript
 import { createStandardRollup, heightUniqueness } from "@sovereign-sdk/web3";
 const rollup = await createStandardRollup();

 const secondResult = await rollup.call(myRuntimeCall, {
   signer: mySigner,
   overrides: {
     uniqueness: await heightUniqueness(rollup),
     /* other overrides... */
   }
 });
 ```
