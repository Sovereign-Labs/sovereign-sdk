# Attester Incentive module

**_This module is a placeholder for the logic incentivizing attesters and challengers. This is the full node implementation of the optimistic rollup workflow_**

This module implements the logic for processing optimistic rollup attestations and challenges. Such
logic is necessary if you want to reward attesters/challengers or do anything else that's "aware" of attestation and challenge generation inside you state transition function.

This module now implements the complete attestion/challenge verification workflow, as well as the bonding and unbonding processes for attesters and challengers.
