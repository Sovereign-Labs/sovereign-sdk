## Key Patterns

### Generic Spec Pattern

All modules are generic over `Spec`, abstracting cryptographic operations:
```rust
pub struct MyModule<S: sov_modules_api::Spec> {
    #[id]
    pub id: ModuleId,
    #[state]
    pub my_state: StateValue<S::Address, MyData>,
}
```

### Dual-Mode Execution

- **Native Mode**: Direct execution with `DefaultContext`
- **ZK Mode**: Execution within zkVM with `ZkDefaultContext`
- Code gated with `#[cfg(feature = "native")]` runs only in native mode

### State Access Pattern

Modules access state through `WorkingSet<S>` or `TxState<S>`:
```rust
impl<S: sov_modules_api::Spec> MyModule<S> {
    pub fn operation(&self, param: Type, state: &mut impl StateReader<S>) -> Result<Output> {
        unimplemented!()
    }
}