# Avail Sovereign DA adapter (presence)

Presence is a _research-only_ adapter making Avail compatible with the Sovereign SDK.

> **_NOTE:_** None of its code is suitable for production use.

### The DaVerifier Trait

The DaVerifier trait is the simpler of the two core traits. Its job is to take a list of BlobTransactions from a DA layer block
and verify that the list is _complete_ and _correct_. Once deployed in a rollup, the data verified by this trait
will be passed to the state transition function, so non-determinism should be strictly avoided.

The logic inside this trait will get compiled down into your rollup's proof system, so it's important to have a high
degree of confidence in its correctness (upgrading SNARKs is hard!) and think carefully about performance.

At a bare minimum, you should ensure that the verifier rejects...

1. If the order of the blobs in an otherwise valid input is changed
1. If the sender of any of the blobs is tampered with
1. If any blob is omitted
1. If any blob is duplicated
1. If any extra blobs are added

For compatibility, we also recommend (but don't require) that any logic in the `DaVerifier` be able to build with `no_std`.
This maximizes your odds of being compatible with any given zk proof system.

### The DaService Trait

The `DaService` trait is slightly more complicated than the `DaVerifier`. Thankfully, it exists entirely outside of the
rollup's state machine - so it never has to be proven in zk. This means that its perfomance is less critical, and that
upgrading it in response to a vulnerability is much easier.

The job of the `DAService` is to allow the Sovereign SDK's node software to communicate with a DA layer. It has two related
responsibilities. The first is to interact with DA layer nodes via RPC - retrieving data for the rollup as it becomes
available. The second is to process that data into the form expected by the `DaVerifier`. For example, almost all DA layers
provide data in JSON format via RPC - but, parsing JSON in a zk-SNARK would be horribly inefficient. So, the `DaService`
is responsible for both querying the RPC service and transforming its responses into a more useful format.

