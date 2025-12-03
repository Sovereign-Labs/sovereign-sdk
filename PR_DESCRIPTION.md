# Transaction Module Refactoring and V2 Sequencer Data Support

## Summary

Refactors transaction module structure and adds V2 transaction type with sequencer-provided metadata support. V2 transactions enable sequencers to add trusted metadata (like high-precision timestamps) that modules can access via Context.

## Changes

1. **Split transaction module** - Separated version types into `unsigned.rs`, `v0.rs`, `v1.rs`, `v2.rs`
2. **Add V2 transaction type** - New variant with `sequencing_data` field for sequencer metadata
3. **Sequencing data flow** - Flows from transaction → authentication → Context → modules
4. **Timestamp population** - Sequencer automatically adds nanosecond timestamps to V2 transactions
5. **Eliminate authentication duplication** - Moved auth data extraction to version struct methods
6. **Simplify verification flow** - Consolidated common verification steps

## Key Features

### Sequencing Data Infrastructure
- **V2 Transaction**: Contains `sequencing_data: Vec<u8>` field
- **Sequencer Population**: Automatically fills field with high-precision timestamp (nanoseconds)
- **Context Access**: Modules get sequencing_data via `context.sequencing_data()`
- **Security Model**: Only registered sequencers can set sequencing_data (not signed by users)

### Data Flow
```
User sends V2 tx with empty sequencing_data
    ↓
Sequencer adds timestamp before hash computation
    ↓
Transaction hash includes sequencing_data
    ↓
Authentication extracts into AuthorizationData
    ↓
resolve_context() populates Context
    ↓
Modules access via context.sequencing_data()
```

## Usage Example

```rust
impl<S: Spec> Module for MyModule<S> {
    fn call(
        &mut self,
        message: Self::CallMessage,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        // Access sequencer timestamp for V2 transactions
        if let Some(data) = context.sequencing_data() {
            let timestamp_nanos = u128::from_le_bytes(data[..16].try_into()?);
            // Use for time-sensitive operations...
        }
        Ok(())
    }
}
```

## Impact

- Net line changes: +688 / -455 (includes new functionality and refactoring)
- **Breaking change**: Code matching on `Transaction` needs `V2` arm
- Easier to add future transaction versions
- More maintainable authentication flow
- Enables high-precision timing for modules

## Security Properties

✅ Sequencing data **NOT signed by users** (excluded from signature verification)
✅ Sequencing data **included in transaction hash** (for uniqueness/replay protection)
✅ Only **registered sequencers** can populate sequencing_data (unregistered cannot)
