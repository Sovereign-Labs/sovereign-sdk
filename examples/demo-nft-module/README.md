# How to Create a New Module Using the Module System

## Understanding the Module System

The Sovereign Software Development Kit (SDK) includes a [Module System](../../module-system/README.md),
which serves as a catalog of concrete and opinionated implementations for the rollup interface.
These modules are the fundamental building blocks of a rollup and include:

* **Protocol-level logic**: This includes elements such as account management, state management logic,
  APIs for other modules, and macros for generating RPC. It provides the blueprint for your rollup.
* **Application-level logic**: This is akin to smart contracts on Ethereum or pallets on Polkadot.
  These modules often use state, modules-API, and macros modules to simplify their development and operation.

## Creating a Non-Fungible Token (NFT) Module

In this tutorial, we will focus on developing an application-level module. Users of this module will be able to mint
unique tokens, transfer them to each other, or burn them. Users can also check the ownership of a particular token. For
simplicity, each token represents only an ID and won't hold any metadata.

# Getting Started

## Structure and dependencies

The Sovereign SDK provides [module-template](../../module-system/module-implementations/module-template/README.md).
It provides a boilerplate which can be customised.
The purpose of each rust module is explained later.

```

├── Cargo.toml
├── README.md
└── src
    ├── call.rs
    ├── genesis.rs
    ├── lib.rs
    ├── query.rs
    └── tests.rs
```

Here are defining basic dependencies in `Cargo.toml` that module needs to get started:

```toml
[dependencies]
anyhow = { anyhow = "1.0.62" }
sov-modules-api = { git = "https://github.com/Sovereign-Labs/sovereign.git", branch = "main", default-features = false }
sov-modules-macros = { git = "https://github.com/Sovereign-Labs/sovereign.git", branch = "main" }
```

## Establishing the Root Module Structure

A module is a distinct crate that implements the `sov_modules_api::Module` trait
and defines its own change logic based on input messages.

## Module definition

NFT module is defined as the following:

```rust
use sov_modules_api::Context;
use sov_modules_macros::ModuleInfo;

#[derive(ModuleInfo, Clone)]
pub struct NonFungibleToken<C: Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub(crate) admin: sov_state::StateValue<C::Address>,

    #[state]
    pub(crate) owners: sov_state::StateMap<u64, C::Address>,

    // If the module needs to refer to another module
    // #[module]
    // pub(crate) bank: bank::Bank<C>,
}
```

This module includes:

1. **Address**: Every module must have an address, like a smart contract address in Ethereum. This ensures that:
    - The module address is unique.
    - The private key that generates this address is unknown.
2. **State** attributes: In this case, the state attributes are the admin's address and a map of token IDs to owner
   addresses.
   For simplicity, the token ID is an u64.
3. **Optional module reference**: This is used if the module needs to refer to another module.

## State and Context

### State

`State` values are stored in a Merkle Tree and can be read and written from there.
The module struct itself doesn't hold values but indicates what values it operates in a working set.
The `WorkingSet` populates the data.
The provided state implementation from the `sov-state` crate uses
the [Jellyfish Merkle Tree](https://github.com/penumbra-zone/jmt) (JMT).

The state operates in full node and zero-knowledge modes.
In full node mode, the entire Merkle tree is maintained and modified, while in zero-knowledge mode,
the state only provides access to Merkle proofs for leaves that were modified in a batch.

### Context

The `Context` trait allows the runtime to pass verified data to modules during execution.
Currently, the only required method in Context is sender(), which returns the address of the individual who initiated
the transaction (the signer).

Context also inherits the Spec trait, which defines the concrete types used by the rollup for Hashing, persistent data
Storage, digital Signatures, and Addresses. The Spec trait allows rollups to easily tailor themselves to different ZK
VMs. By being generic over a Spec, a rollup can ensure that any potentially SNARK-unfriendly cryptography can be easily
swapped out.

# Implementing `sov_modules_api::Module` trait

Before we start implementing the `Module` trait, there are several preparatory steps to take:

1. Add new dependencies:
   [serde](https://serde.rs/),
   [borsh](https://github.com/near/borsh-rs),
   [sov-state](../../module-system/sov-state/README.md)
2. Define a 'native' feature flag to separate logic that isn't needed in zero-knowledge mode.
3. Define `Call` and `Query` messages.
4. Define `Config`.

## Preparation

1. Define `native` feature in `Cargo.toml`:
   ```toml
   [dependencies]
   anyhow = "1.0.62"
   borsh = { version = "0.10.3", features = ["bytes"] }
   serde = { version = "1", features = ["derive"] }
   serde_json = "1"

    sov-modules-api = { git = "https://github.com/Sovereign-Labs/sovereign.git", branch = "main", default-features = false }
    sov-modules-macros = { git = "https://github.com/Sovereign-Labs/sovereign.git", branch = "main" }
    sov-state = { git = "https://github.com/Sovereign-Labs/sovereign.git", branch = "main", default-features = false }

    [features]
    default = ["native"]
    serde = ["dep:serde", "dep:serde_json"]
    native = ["serde", "sov-state/native", "sov-modules-api/native"]
    ```

   This step is necessary to optimize the module for execution in ZK mode, where none of the RPC-related logic is
   needed.
   Zero Knowledge mode uses a different serialization format, so serde is not needed.
   The `sov-state` module maintains same logic, so its `native` flag only enabled in that case.

2. Define `Call` and `Query` messages: `Call` messages are used to change the state of the module, while `Query`
   messages are used to read the state of the module.
    ```rust
    use sov_modules_api::Context;

    #[cfg_attr(feature = "native", derive(serde::Serialize), derive(serde::Deserialize))]
    #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
    pub enum CallMessage<C: Context> {
        Mint {
            /// The id of new token. Caller is an owner
            id: u64
        },
        Transfer {
            /// The address to which the token will be transferred.
            to: C::Address,
            /// The token id to transfer
            id: u64,
        },
        Burn {
            id: u64,
        }
    }


    /// This enumeration responsible for querying the nft module.
    #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
    pub enum QueryMessage {
        GetOwner { token_id: u64 },
    }
    ```
       The Query message is only used in native mode, as it doesn't change the state and therefore isn't used in the state
       transition function.

3. Define Config. In this case, config will contain admin and initial tokens:
   ```rust
   pub struct NonFungibleTokenConfig<C: Context> {
       pub admin: C::Address,
       pub owners: Vec<(u64, C::Address)>,
   }
   ```

# Stub implementation of the Module trait

Plug together all types and features

```rust
impl<C: Context> Module for NonFungibleToken<C> {
    type Context = C;

    type Config = NonFungibleTokenConfig<C>;

    type CallMessage = call::CallMessage<C>;

    #[cfg(feature = "native")]
    type QueryMessage = query::QueryMessage;

    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn call(
        &self,
        _msg: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse, Error> {
        Ok(CallResponse::default())
    }

    #[cfg(feature = "native")]
    fn query(
        &self,
        _msg: Self::QueryMessage,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> QueryResponse {
        QueryResponse::default()
    }
}
```

# Implementing state change logic

## Initialization

Initialization is performed by the genesis method,
which takes a config argument specifying the initial state to configure.
Since it modifies state, genesis also takes a working set as an argument.
Genesis is called only once, during the rollup deployment.

```rust
impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.admin.set(config.admin.clone(), working_set);
        for (id, owner) in config.owners.iter() {
            if self.owners.get(id, working_set).is_some() {
                bail!("Token id {} already exists", id);
            }
            self.owners.set(id, owner.clone(), working_set);
        }
        Ok(())
    }
}
```

And then adding this piece to trait implementation:

```rust
impl<C: Context> Module for NonFungibleToken<C> {
    // ...
    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }
}
```

## Call message

First, need to implement actual logic of handling different cases, let's add `mint`, `transfer` and `burn` methods:

```rust

impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn mint(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        if self.owners.get(&id, working_set).is_some() {
            bail!("Token with id {} already exists", id);
        }

        self.owners.set(&id, context.sender().clone(), working_set);
        Ok(CallResponse::default())
    }

    pub(crate) fn transfer(
        &self,
        id: u64,
        to: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token_owner = match self.owners.get(&id, working_set) {
            None => {
                bail!("Token with id {} does not exist", id);
            }
            Some(owner) => owner,
        };
        if &token_owner != context.sender() {
            bail!("Only token owner can transfer token");
        }
        self.owners.set(&id, to, working_set);
        Ok(CallResponse::default())
    }

    pub(crate) fn burn(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token_owner = match self.owners.get(&id, working_set) {
            None => {
                bail!("Token with id {} does not exist", id);
            }
            Some(owner) => owner,
        };
        if &token_owner != context.sender() {
            bail!("Only token owner can burn token");
        }
        self.owners.remove(&id, working_set);
        Ok(CallResponse::default())
    }
}
```

And then map it in the trait implementation:

```rust
impl<C: Context> Module for NonFungibleToken<C> {
    // ...

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse, Error> {
        let call_result = match msg {
            call::CallMessage::Mint { id } => self.mint(id, context, working_set),
            call::CallMessage::Transfer { to, id } => self.transfer(id, to, context, working_set),
            call::CallMessage::Burn { id } => self.burn(id, context, working_set),
        };
        Ok(call_result?)
    }
}
```

## Query Messages

The same approach follows for the query:

```rust
impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn get_owner(
        &self,
        token_id: u64,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> OwnerResponse<C> {
        OwnerResponse {
            owner: self.owners.get(&token_id, working_set),
        }
    }
}
```

And then plug it in trait implementation:

```rust
impl<C: Context> Module for NonFungibleToken<C> {
    // ...
    #[cfg(feature = "native")]
    fn query(
        &self,
        msg: Self::QueryMessage,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> QueryResponse {
        match msg {
            query::QueryMessage::GetOwner { token_id } => {
                let response = serde_json::to_vec(&self.get_owner(token_id, working_set)).unwrap();
                QueryResponse { response }
            }
        }
    }
}
```

# Testing

To make sure that module is implemented correctly, integration tests are recommended, so all public APIs work as
expected.

For testing temporary storage is needed, so `temp` feature for `sov-state` module is required:

```toml
[dev-dependencies]
sov-state = { git = "https://github.com/Sovereign-Labs/sovereign.git", branch = "main", features = ["temp"] }
```

Here is boilerplate for NFT module integration tests

```rust
use demo_nft_module::call::CallMessage;
use demo_nft_module::query::{OwnerResponse, QueryMessage};
use demo_nft_module::{NonFungibleToken, NonFungibleTokenConfig};
use serde::de::DeserializeOwned;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context, Hasher, Module, ModuleInfo, Spec};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

pub type C = DefaultContext;
pub type Storage = ProverStorage<DefaultStorageSpec>;

pub fn generate_address(key: &str) -> <C as Spec>::Address {
    let hash = <C as Spec>::Hasher::hash(key.as_bytes());
    Address::from(hash)
}

pub fn query_and_deserialize<R: DeserializeOwned>(
    nft: &NonFungibleToken<C>,
    query: QueryMessage,
    working_set: &mut WorkingSet<Storage>,
) -> R {
    let response = nft.query(query, working_set);
    serde_json::from_slice(&response.response).expect("Failed to deserialize response json")
}

#[test]
#[ignore = "Not implemented yet"]
fn genesis_and_mint() {}

#[test]
#[ignore = "Not implemented yet"]
fn transfer() {}

#[test]
#[ignore = "Not implemented yet"]
fn burn() {}
```

Here's an example of setting up a module and calling its methods:

```rust
#[test]
fn transfer() {
    // Preparation
    let admin = generate_address("admin");
    let admin_context = C::new(admin.clone());
    let owner1 = generate_address("owner2");
    let owner1_context = C::new(owner1.clone());
    let owner2 = generate_address("owner2");
    let config: NonFungibleTokenConfig<C> = NonFungibleTokenConfig {
        admin: admin.clone(),
        owners: vec![(0, admin.clone()), (1, owner1.clone()), (2, owner2.clone())],
    };
    let mut working_set = WorkingSet::new(ProverStorage::temporary());
    let nft = NonFungibleToken::new();
    nft.genesis(&config, &mut working_set).unwrap();

    let transfer_message = CallMessage::Transfer {
        id: 1,
        to: owner2.clone(),
    };

    // admin cannot transfer token of the owner1
    let transfer_attempt = nft.call(transfer_message.clone(), &admin_context, &mut working_set);

    assert!(transfer_attempt.is_err());
    /// ... rest of the tests
}
```

# Plugging in the rollup

Now this module can be added to rollup's Runtime:

```rust
#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: Context> {
    #[allow(unused)]
    sequencer: sequencer::Sequencer<C>,

    #[allow(unused)]
    bank: bank::Bank<C>,

    #[allow(unused)]
    nft: nft::NonFungibleToken<C>,
}
```

And then this runtime can be used in the State Transition Function runner to execute transactions.
Here's an example of how to do it with `AppTemplate` from `sov-app-template`:

```rust
    fn new(runtime_config: Self::RuntimeConfig) -> Self {
    let runtime = Runtime::new();
    let storage = ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
    let tx_verifier = DemoAppTxVerifier::new();
    let tx_hooks = DemoAppTxHooks::new();
    let app: AppTemplate<
        ZkDefaultContext,
        DemoAppTxVerifier<ZkDefaultContext>,
        Runtime<ZkDefaultContext>,
        DemoAppTxHooks<ZkDefaultContext>,
        Vm,
    > = AppTemplate::new(storage, runtime, tx_verifier, tx_hooks);
    Self(app)
}
```

`AppTemplate` uses runtime to dispatch call during execution of `apply_batch` method.
More details on how to setup rollup is available in [demo-rollup documentation](../demo-rollup/README.md)
