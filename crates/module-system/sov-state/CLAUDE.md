# State Management

Low-level storage and state management interfaces for Sovereign SDK modules.

## Core Components

### Storage Trait
Authenticated state storage interface:
- Merkle tree-based with proofs
- Support for historical queries
- Witness generation for ZK proving
- Both native and ZK-compatible implementations

### Key Storage Implementations

1. **NOMT (New Optimized Merkle Tree)** (`nomt/`):
   - High-performance merkle tree optimized for rollups
   - Efficient for frequent updates
   - Located in separate `nomt` module

2. **JMT (Jellyfish Merkle Tree)**:
   - Re-exported from `jmt` crate
   - Standard sparse merkle tree implementation
   - Good baseline performance

### Core Types

- **`StorageRoot`**: Merkle root representing state at a point in time
- **`SparseMerkleProof`**: Cryptographic proofs of state inclusion
- **`ArrayWitness`/`Witness`**: Tracks state access for proof generation
- **`TypeErasedEvent`**: Event storage abstraction

### Storage Variants

1. **`ProverStorage`** (native feature):
   - For proof generation mode
   - Tracks all state access for witnesses

2. **`ZkStorage`**:
   - For ZK execution mode
   - Minimal interface for circuit execution

3. **`SequencerState`** (native feature):
   - Sequencer-specific state management
   - Extended functionality for block production

### Caching and Performance

- **`cache`**: State caching layer for performance
- **`pinned_cache`**: Persistent cache with pinning
- **`DEFAULT_CACHE_CAPACITY`**: 32 entries initial capacity

### Namespaces

State isolation system:
- Each module gets unique namespace prefix
- Prevents cross-module state conflicts
- Supports state organization and migration
- Compile-time namespace safety

## Key Traits

### MerkleProofSpec
Configures cryptographic primitives:
- Witness accumulation structure
- Hash function (32-byte output required)
- Send + Sync for concurrency

### Storage
Core storage interface:
- Get/set operations with proofs
- Historical state queries
- Root hash management
- Witness generation

## Module Integration

State containers are defined in `sov-modules-api/containers/`:
- **StateMap**: Key-value storage
- **StateVec**: Ordered list storage  
- **StateValue**: Single value storage
- **VersionedStateValue**: Versioned single values

Note: No `StateQueue` or `StateOption` containers exist - only the above four types.