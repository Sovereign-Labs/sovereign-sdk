# Prover Incentives

**_This module is a placeholder for the logic incentivizing provers_**

This module implements the logic for processing proof transactions. Such
logic is necessary if you want to reward provers or do anything else that's "aware" of proof
generation inside you state transition function.

Currently, this module allows provers to register and de-register, and allows the on-chain validation
of proofs from registered provers. If proof validation fails, the offending prover is slashed.

This module does _not_ reward provers - incentives for provers will depend on gas metering, which has
yet to be implemented.
