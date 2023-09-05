# How to Create a New Module Using the Module System

### Understanding the Module System

The Sovereign Software Development Kit (SDK) includes a [Module System](../../module-system/README.md),
which serves as a catalog of concrete and opinionated implementations for the rollup interface.
These modules are the fundamental building blocks of a rollup and include:

- **Protocol-level logic**: This includes elements such as account management, state management logic,
  APIs for other modules, and macros for generating RPC. It provides the blueprint for your rollup.
- **Application-level logic**: This is akin to smart contracts on Ethereum or pallets on Polkadot.
  These modules often use state, modules-API, and macros modules to simplify their development and operation.

### Creating a Non-Fungible Token (NFT) Module

In this tutorial, we will focus on developing an application-level module. Users of this module will be able to mint
unique tokens, transfer them to each other, or burn them. Users can also check the ownership of a particular token. For
simplicity, each token represents only an ID and won't hold any metadata.

## Getting Started

### Structure and dependencies

The Sovereign SDK provides a [module-template](../../module-system/module-implementations/module-template/README.md),
which is boilerplate that can be customized to easily build modules.

```text

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
sov-modules-api = { git = "https://github.com/Sovereign-Labs/sovereign-sdk.git", branch = "stable", features = ["macros"] }
```

### Establishing the Root Module Structure

A module is a distinct crate that implements the `sov_modules_api::Module` trait. Each module
has private state, which it updates in response to input messages.

### Module definition

NFT module is defined as the following:

```rust
#[derive(sov_modules_api::ModuleInfo, Clone)]
pub struct NonFungibleToken<C: sov_modules_api::Context> {
    #[address]
    address: C::Address,

    #[state]
    admin: sov_state::StateValue<C::Address>,

    #[state]
    owners: sov_state::StateMap<u64, C::Address>,

    // If the module needs to refer to another module
    // #[module]
    // bank: sov_bank::Bank<C>,
}
```

This module includes:

1. **Address**: Every module must have an address, like a smart contract address in Ethereum. This ensures that:
   - The module address is unique.
   - The private key that generates this address is unknown.
2. **State attributes**: In this case, the state attributes are the admin's address and a map of token IDs to owner
   addresses.
   For simplicity, the token ID is an u64.
3. **Optional module reference**: This is used if the module needs to refer to another module.

### State and Context

#### State

`#[state]` values declared in a module are not physically stored in the module. Instead, the module definition
simply declares the _types_ of the values that it will access. The values themselves live in a special struct
called a `WorkingSet`, which abstracts away the implementation details of storage. In the default implementation, the actual state values live in a [Jellyfish Merkle Tree](https://github.com/penumbra-zone/jmt) (JMT).
This separation between functionality (defined by the `Module`) and state (provided by the `WorkingSet`) explains
why so many module methods take a `WorkingSet` as an argument.

#### Context

The `Context` trait allows the runtime to pass verified data to modules during execution.
Currently, the only required method in Context is sender(), which returns the address of the individual who initiated
the transaction (the signer).

Context also inherits the Spec trait, which defines the concrete types used by the rollup for Hashing, persistent data
Storage, digital Signatures, and Addresses. The Spec trait allows rollups to easily tailor themselves to different ZK
VMs. By being generic over a Spec, a rollup can ensure that any potentially SNARK-unfriendly cryptography can be easily
swapped out.

## Implementing `sov_modules_api::Module` trait

### Preparation

Before we start implementing the `Module` trait, there are several preparatory steps to take:

1.  Define `native` feature in `Cargo.toml` and add additional dependencies:

    ```toml
    [dependencies]
    anyhow = "1.0.62"
    borsh = { version = "0.10.3", features = ["bytes"] }
    serde = { version = "1", features = ["derive"] }
    serde_json = "1"

    sov-modules-api = { git = "https://github.com/Sovereign-Labs/sovereign-sdk.git", branch = "stable", default-features = false, features = ["macros"] }
    sov-state = { git = "https://github.com/Sovereign-Labs/sovereign-sdk.git", branch = "stable", default-features = false }

    [features]
    default = ["native"]
    serde = ["dep:serde", "dep:serde_json"]
    native = ["serde", "sov-state/native", "sov-modules-api/native"]
    ```

    This step is necessary to optimize the module for execution in ZK mode, where none of the RPC-related logic is
    needed.
    Zero Knowledge mode uses a different serialization format, so serde is not needed.
    The `sov-state` module maintains the same logic, so its `native` flag is only enabled in that case.

2.  Define `Call` messages, which are used to change the state of the module:

    ```rust
    // in call.rs
    #[cfg_attr(feature = "native", derive(serde::Serialize), derive(serde::Deserialize))]
    #[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
    pub enum CallMessage<C: sov_modules_api::Context> {
        Mint {
            /// The id of new token. Caller is an owner
            id: u64,
        },
        Transfer {
            /// The address to which the token will be transferred.
            to: C::Address,
            /// The token id to transfer.
            id: u64,
        },
        Burn {
            id: u64,
        }
    }
    ```

    As you can see, we derive the `borsh` serialization format for these messages. Unlike most serialization libraries,
    `borsh` guarantees that all messages have a single "canonical" serialization, which makes it easy to reliably
    hash and compare serialized messages.

3.  Create a `Config` struct for the genesis configuration. In this case, the admin address and initial token distribution
    are configurable:

    ```rust
    // in lib.rs
    pub struct NonFungibleTokenConfig<C: sov_modules_api::Context> {
        pub admin: C::Address,
        pub owners: Vec<(u64, C::Address)>,
    }
    ```

## Stub implementation of the Module trait

Plugging together all types and features, we get this `Module` trait implementation in `lib.rs`:

```rust, ignore
impl<C: sov_modules_api::Context> Module for NonFungibleToken<C> {
    type Context = C;
    type Config = NonFungibleTokenConfig<C>;
    type CallMessage = CallMessage<C>;

    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<(), Error> {
        Ok(())
    }

    fn call(
        &self,
        _msg: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<sov_modules_api::CallResponse, Error> {
        Ok(sov_modules_api::CallResponse::default())
    }
}
```

## Implementing state change logic

### Initialization

Initialization is performed by the `genesis` method,
which takes a config argument specifying the initial state to configure.
Since it modifies state, `genesis` also takes a working set as an argument.
`Genesis` is called only once, during the rollup deployment.

```rust, ignore
use sov_state::WorkingSet;

// in lib.rs
impl<C: sov_modules_api::Context> sov_modules_api::Module for NonFungibleToken<C> {
    type Context = C;
    type Config = NonFungibleTokenConfig<C>;
    type CallMessage = CallMessage<C>;
    
    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }
}

// in genesis.rs
impl<C: sov_modules_api::Context> NonFungibleToken<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<()> {
        self.admin.set(&config.admin, working_set);
        for (id, owner) in config.owners.iter() {
            if self.owners.get(id, working_set).is_some() {
                anyhow::bail!("Token id {} already exists", id);
            }
            self.owners.set(id, owner, working_set);
        }
        Ok(())
    }
}
```

### Call message

First, we need to implement actual logic of handling different cases. Let's add `mint`, `transfer` and `burn` methods:

```rust, ignore
use sov_state::WorkingSet;

impl<C: sov_modules_api::Context> NonFungibleToken<C> {
    pub(crate) fn mint(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<sov_modules_api::CallResponse> {
        if self.owners.get(&id, working_set).is_some() {
            bail!("Token with id {} already exists", id);
        }

        self.owners.set(&id, context.sender(), working_set);

        working_set.add_event("NFT mint", &format!("A token with id {id} was minted"));
        Ok(sov_modules_api::CallResponse::default())
    }

    pub(crate) fn transfer(
        &self,
        id: u64,
        to: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<sov_modules_api::CallResponse> {
        let token_owner = match self.owners.get(&id, working_set) {
            None => {
                anyhow::bail!("Token with id {} does not exist", id);
            }
            Some(owner) => owner,
        };
        if &token_owner != context.sender() {
            anyhow::bail!("Only token owner can transfer token");
        }
        self.owners.set(&id, &to, working_set);
        working_set.add_event(
            "NFT transfer",
            &format!("A token with id {id} was transferred"),
        );
        Ok(sov_modules_api::CallResponse::default())
    }

    pub(crate) fn burn(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<sov_modules_api::CallResponse> {
        let token_owner = match self.owners.get(&id, working_set) {
            None => {
                anyhow::bail!("Token with id {} does not exist", id);
            }
            Some(owner) => owner,
        };
        if &token_owner != context.sender() {
            anyhow::bail!("Only token owner can burn token");
        }
        self.owners.remove(&id, working_set);

        working_set.add_event("NFT burn", &format!("A token with id {id} was burned"));
        Ok(sov_modules_api::CallResponse::default())
    }
}
```

And then make them accessible to users via the `call` function:

```rust, ignore
impl<C: sov_modules_api::Context> sov_modules_api::Module for NonFungibleToken<C> {
    type Context = C;
    type Config = NonFungibleTokenConfig<C>;

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        let call_result = match msg {
            CallMessage::Mint { id } => self.mint(id, context, working_set),
            CallMessage::Transfer { to, id } => self.transfer(id, to, context, working_set),
            CallMessage::Burn { id } => self.burn(id, context, working_set),
        };
        Ok(call_result?)
    }
}
```

### Enabling Queries

We also want other modules to be able to query the owner of a token, so we add a public method for that.
This method is only available to other modules: it is not currently exposed via RPC.

```rust, ignore
use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::Context;
use sov_state::WorkingSet;

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
/// Response for `getOwner` method
pub struct OwnerResponse<C: Context> {
    /// Optional owner address
    pub owner: Option<C::Address>,
}

#[rpc_gen(client, server, namespace = "nft")]
impl<C: sov_modules_api::Context> NonFungibleToken<C> {
    #[rpc_method(name = "getOwner")]
    pub fn get_owner(
        &self,
        token_id: u64,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<OwnerResponse<C>> {
        Ok(OwnerResponse {
            owner: self.owners.get(&token_id, working_set),
        })
    }
}
```

## Testing

Integration tests are recommended to ensure that the module is implemented correctly. This helps confirm
that all public APIs function as intended.

Temporary storage is needed for testing, so we enable the `temp` feature of `sov-state` as a `dev-dependency`

```toml,text
[dev-dependencies]
sov-state = { git = "https://github.com/Sovereign-Labs/sovereign-sdk.git", branch = "stable", features = ["temp"] }
```

Here is some boilerplate for NFT module integration tests:

```rust
use demo_nft_module::{CallMessage, NonFungibleToken, NonFungibleTokenConfig, OwnerResponse};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context, Module};
use sov_rollup_interface::stf::Event;
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

pub type C = DefaultContext;
pub type Storage = ProverStorage<DefaultStorageSpec>;


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
    let admin = generate_address::<C>("admin");
    let admin_context = C::new(admin.clone());
    let owner1 = generate_address::<C>("owner2");
    let owner1_context = C::new(owner1.clone());
    let owner2 = generate_address::<C>("owner2");
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
    // ... rest of the tests
}
```

## Plugging in the rollup

Now this module can be added to rollup's `Runtime`:

```rust, ignore
use sov_modules_api::{DispatchCall, Genesis, MessageCodec};

#[derive(Genesis, DispatchCall, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: sov_modules_api::Context> {
    #[allow(unused)]
    sequencer: sov_sequencer_registry::Sequencer<C>,

    #[allow(unused)]
    bank: sov_bank::Bank<C>,

    #[allow(unused)]
    nft: demo_nft_module::NonFungibleToken<C>,
}
```
