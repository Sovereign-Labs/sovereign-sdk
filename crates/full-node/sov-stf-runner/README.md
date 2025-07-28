# Sov-Stf-Runner

Generic logic for running the rollup.

### StateTransitionRunner

The `StateTransitionRunner` combines the `StateTransitionFunction` with `DaService` and runs the rollup by invoking the
blob processing logic on blocks obtained from `DaService`.
Additionally, it allows the initiation of an HTTP server with externally defined API methods.
