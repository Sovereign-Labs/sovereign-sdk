/// Contains the `cycle-tracker` macro which can be used to annotate functions that run inside a zkvm
///
/// ```rust,ignore
/// #[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
/// fn begin_slot(
///     &mut self,
///     slot_data: &impl SlotData<Cond = Cond>,
///     witness: <Self as StateTransitionFunction<Vm, B>>::Witness,
/// ) {
///     let state_checkpoint = StateCheckpoint::with_witness(self.current_storage.clone(), witness);
///
///     let mut working_set = state_checkpoint.to_revertable();
///
///     self.runtime.begin_rollup_block_hook(slot_data, &mut working_set);
///
///     self.checkpoint = Some(working_set.checkpoint());
/// }
#[cfg(feature = "bench")]
pub use sov_modules_macros::cycle_tracker;
/// This macro is used to annotate functions that we want to track the usage of gas constants within the SDK.
/// The purpose of the this macro is to measure how times different gas constants have been used within an annotated function
/// to be able to estimate constant values.
///
/// One can add attribute arguments to this macro. Arguments should specify the name of the function inputs
/// to track as metadata. For instance:
///
/// ```rust
/// use sov_modules_macros::track_gas_constants_usage;
///
/// #[track_gas_constants_usage(_input)]
/// fn test_metrics(_input: &mut u64) {
///     
/// }
/// ```
///
/// Will add `input={input_value}` as a metric metadata when collecting gas constant usage here.
#[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
pub use sov_modules_macros::track_gas_constants_usage;
/// Derives the [`DispatchCall`] trait for the underlying
/// type.
///
/// ```rust,no_run
/// use sov_modules_api::{DaSpec, DispatchCall, Module, Spec};
/// use sov_bank::Bank;
/// use sov_sequencer_registry::SequencerRegistry;
///
/// struct MyRuntime<S: Spec> {
///   pub bank: Bank<S>,
///   pub sequencer_registry: SequencerRegistry<S>,
/// }
///
/// // Applying #[derive(DispatchCall)] to MyRuntime generates the following code:
/// #[allow(non_camel_case_types)]
/// #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, borsh::BorshSerialize, borsh::BorshDeserialize)]
/// #[derive(
///    sov_modules_api::prelude::strum::EnumDiscriminants,
///    sov_modules_api::prelude::strum::VariantNames,
///    sov_modules_api::prelude::strum::EnumTryAs,
///    sov_modules_api::prelude::strum::IntoStaticStr,
///    sov_modules_api::prelude::strum::AsRefStr,
///    sov_modules_api::macros::UniversalWallet,
/// )]
/// #[strum_discriminants(derive(
///    sov_modules_api::prelude::strum::VariantNames,
///    sov_modules_api::prelude::strum::VariantArray,
///    sov_modules_api::prelude::strum::EnumString,
///    sov_modules_api::prelude::strum::IntoStaticStr,
///    sov_modules_api::prelude::strum::AsRefStr,
///    sov_modules_api::prelude::schemars::JsonSchema,
///    sov_modules_api::macros::UniversalWallet,
/// ))]
/// #[serde(rename_all = "snake_case")]
/// pub enum RuntimeCall<S: Spec> {
///   bank(<Bank::<S> as Module>::CallMessage),
///   sequencer_registry(<SequencerRegistry::<S> as Module>::CallMessage),
/// }
///
///
/// impl<S: Spec> sov_modules_api::NestedEnumUtils for RuntimeCall<S> {
///     type Discriminants = RuntimeCallDiscriminants;
///  
///     fn discriminant(&self) -> Self::Discriminants {
///         self.into()
///     }
///   
///     fn raw_contents(&self) -> &dyn std::any::Any {
///         match self {
///             Self::bank(inner) => inner,
///             Self::sequencer_registry(inner) => inner,
///         }
///     }
///  }
///
/// impl<S: Spec> DispatchCall for MyRuntime<S> {
///   type Spec = S;
///
///   type Decodable = RuntimeCall<S>;
///
/// // -- Method bodies elided for brevity --
/// # fn encode(decodable: &Self::Decodable) -> Vec<u8> {
/// #     borsh::to_vec(decodable).unwrap()
/// # }
/// #
/// #
/// # fn dispatch_call<I: sov_modules_api::StateProvider<S>>(
/// #     &mut self,
/// #     message: Self::Decodable,
/// #     state: &mut sov_modules_api::WorkingSet<Self::Spec, I>,
/// #     context: &sov_modules_api::Context<Self::Spec>,
/// # ) -> Result<(), sov_modules_api::ModuleError> {
/// #   Ok(Default::default())
/// # }
/// //Returns the ID of the dispatched module.
/// # fn module_id(&self, _message: &Self::Decodable) -> &sov_modules_api::ModuleId {
/// #   use sov_modules_api::ModuleInfo;
/// #   self.bank.id()
/// # }
/// # fn module_info(
/// #     &self,
/// #     discriminant: <Self::Decodable as ::sov_modules_api::NestedEnumUtils>::Discriminants,
/// # ) -> &dyn ::sov_modules_api::ModuleInfo<Spec = Self::Spec> {
/// #     todo!()
/// # }
/// }
/// # fn main() {}
/// ```
///
/// ## Attribute: `#[dispatch_call(no_default_attrs)]`
///
/// This attribute disables all of the default attributes that are applied to the generated
/// enum. This is typically useful if you want to provide a custom implementation of a serializer.
///
/// The current set of default attributes is:
///
/// `#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, borsh::BorshSerialize, borsh::BorshDeserialize)]`
/// `#[derive(sov_modules_api::prelude::strum::EnumDiscriminants, sov_modules_api::prelude::strum::VariantNames, sov_modules_api::prelude::strum::EnumTryAs, sov_modules_api::prelude::strum::IntoStaticStr, sov_modules_api::prelude::strum::AsRefStr)]`
/// `#[strum_discriminants(derive(sov_modules_api::prelude::strum::VariantNames, sov_modules_api::prelude::strum::VariantArray, sov_modules_api::prelude::strum::EnumString, sov_modules_api::prelude::strum::IntoStaticStr, sov_modules_api::prelude::strum::AsRefStr))]`
/// `#[serde(rename_all = "snake_case")]`
///
///
/// ## Attribute: `#[dispatch_call({attr})]`
///
/// This attribute allows you to provide custom attributes to the generated enum.
///
/// ```rust
/// use sov_modules_api::{DaSpec, DispatchCall, Spec};
/// use sov_bank::Bank;
/// use sov_sequencer_registry::SequencerRegistry;
///
/// #[derive(DispatchCall)]
/// #[dispatch_call(serde(untagged))]
/// struct MyRuntime<S: Spec> {
///   pub bank: Bank<S>,
///   pub sequencer_registry: SequencerRegistry<S>,
/// }
/// # fn main() {}
/// ```
pub use sov_modules_macros::DispatchCall;
/// Derives the `<runtime_name>Event` enum for a given runtime.
///
/// ```rust
/// use sov_modules_api::{Event, Module, Spec};
/// use sov_bank::Bank;
/// use sov_sequencer_registry::SequencerRegistry;
///
/// struct Runtime<S: Spec> {
///   pub bank: Bank<S>,
///   pub sequencer_registry: SequencerRegistry<S>,
/// }
///
/// // Applying #[derive(Event)] to MyRuntime generates the following code:
/// #[allow(non_camel_case_types)]
/// #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, borsh::BorshSerialize, borsh::BorshDeserialize)]
/// #[serde(untagged, bound = "")]
/// pub enum RuntimeEvent<S: Spec> {
///   bank(<Bank::<S> as Module>::Event),
///   sequencer_registry(<SequencerRegistry::<S> as Module>::Event),
/// }
/// ```
///
/// ## Attribute: `#[event(no_default_attrs)]`
///
/// This attribute disables all of the default attributes that are applied to the generated
/// enum. This is typically useful if you want to provide a custom implementation of a serializer.
///
/// The current default attributes are:
///
/// - `#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, borsh::BorshSerialize, borsh::BorshDeserialize, schemars::JsonSchema)]`
/// - `#[derive(sov_modules_api::prelude::strum::EnumDiscriminants, sov_modules_api::prelude::strum::VariantNames, sov_modules_api::prelude::strum::EnumTryAs, sov_modules_api::prelude::strum::IntoStaticStr, sov_modules_api::prelude::strum::AsRefStr)]`
/// - `#[strum_discriminants(derive(sov_modules_api::prelude::strum::VariantNames, sov_modules_api::prelude::strum::VariantArray, sov_modules_api::prelude::strum::EnumString, sov_modules_api::prelude::strum::IntoStaticStr, sov_modules_api::prelude::strum::AsRefStr))]`
/// - `#[serde(untagged, bound = "", rename_all="snake_case")]`
///
///
/// ## Attribute: `#[event({attr})]`
///
/// This attribute allows you to provide attributes to the generated enum.
///
/// ```rust
/// use sov_modules_api::{DaSpec, Event, Spec};
/// use sov_bank::Bank;
/// use sov_sequencer_registry::SequencerRegistry;
///
/// #[derive(Event)]
/// #[event(serde(deny_unknown_fields), borsh(use_discriminant = false))]
/// struct Runtime<S: Spec> {
///   pub bank: Bank<S>,
///   pub sequencer_registry: SequencerRegistry<S>,
/// }
/// # fn main() {}
/// ```
pub use sov_modules_macros::Event;
/// Derives the [`Genesis`](trait.Genesis.html) trait for the underlying runtime
/// `struct`.
pub use sov_modules_macros::Genesis;
/// Derives the [`BlockHooks`](trait.BlockHooks.html) trait for the underlying runtime
pub use sov_modules_macros::Hooks;
pub use sov_modules_macros::MessageCodec;
/// Derives the [`ModuleInfo`] trait for the underlying `struct`.
///
/// The underlying type must respect the following conditions, or compilation
/// will fail:
/// - It must be a named `struct`. Tuple `struct`s, `enum`s, and others are
///   not supported.
/// - It must have *exactly one* field with the `#[id]` attribute. This field
///   represents the **module id**.
/// - All other fields must have either the `#[state]` or `#[module]` attribute.
///   - `#[state]` is used for state members.
///   - `#[module]` is used for module members.
///
/// In addition to implementing [`ModuleInfo`], this macro will
/// also generate so-called "prefix" methods.
///
/// ## Example
///
/// ```
/// use sov_modules_api::{Spec, ModuleId, ModuleInfo, StateMap};
///
/// #[derive(Clone, ModuleInfo)]
/// struct TestModule<S: Spec> {
///     #[id]
///     id: ModuleId,
///
///     #[state]
///     pub state_map: StateMap<String, u32>,
///
///     #[phantom]
///     phantom: std::marker::PhantomData<S>,
/// }
///
/// // You can then get the prefix of `state_map` like this:
/// fn get_prefix<S: Spec>(some_storage: S::Storage) {
///     let test_struct = TestModule::<S>::default();
///     let prefix1 = test_struct.state_map.prefix();
/// }
/// ```
pub use sov_modules_macros::ModuleInfo;
/// Derives [`HasRestApi`](crate::rest::HasRestApi) for modules.
///
/// REST APIs generated with this proc-macro will serve static metadata
/// about the module itself, such as:
/// - its name;
/// - its description;
/// - its [`ModuleId`](crate::ModuleId).
///
/// In addition to static metadata, the API also provides access to
/// the module's state items (e.g. [`StateMap`](crate::containers::StateMap))'s
/// values, both at the latest block and at specific block heights.
/// The root path contains a listing of all state items that can be queried
/// through the API.
///
/// ## Attributes: `#[rest_api(skip)]`
///
/// Tells the proc-macro to **NOT** provide access to a specific state item
/// within the module.
///
/// ```
/// use sov_modules_api::prelude::*;
/// use sov_modules_api::{ModuleId, ModuleInfo, StateValue};
///
/// #[derive(Clone, ModuleInfo, ModuleRestApi)]
/// struct MyModule<S: Spec> {
///     #[id]
///     id: ModuleId,
///     /// This state item can't be queried through the API.
///     #[state]
///     #[rest_api(skip)]
///     state_item: StateValue<S::Address>,
/// }
/// # // BEGIN MODULE IMPL, COPY-PASTE-ME (https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#hiding-portions-of-the-example)
/// # impl<S: Spec> sov_modules_api::Module for MyModule<S> {
/// #    type Spec = S;
/// #    type Config = ();
/// #    type CallMessage = ();
/// #    type Event = ();
/// #
/// #    fn call(
/// #        &mut self,
/// #        _msg: Self::CallMessage,
/// #        _context: &Context<Self::Spec>,
/// #        _state: &mut impl sov_modules_api::state::TxState<S>,
/// #    ) -> anyhow::Result<()> {
/// #        unimplemented!()
/// #    }
/// # }
/// # // END MODULE IMPL
/// ```
///
/// ## Attributes: `#[rest_api(include)]`
///
/// Tells the proc-macro that compilation **MUST** fail if the marked state
/// item can't be exposed through the API, e.g. for unsatisfied trait
/// bounds, instead of silently ignoring the item.
///
/// ```
/// use sov_modules_api::prelude::*;
/// use sov_modules_api::{ModuleId, ModuleInfo, StateValue};
///
/// #[derive(Clone, ModuleInfo, ModuleRestApi)]
/// struct MyModule<S: Spec> {
///     #[id]
///     id: ModuleId,
///     /// If someone were to replace `S::Address` with a type that doesn't
///     /// satisfy the necessary trait bounds, the compiler will complain.
///     #[state]
///     #[rest_api(include)]
///     state_item: StateValue<S::Address>,
/// }
/// # // BEGIN MODULE IMPL, COPY-PASTE-ME (https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#hiding-portions-of-the-example)
/// # impl<S: Spec> sov_modules_api::Module for MyModule<S> {
/// #    type Spec = S;
/// #    type Config = ();
/// #    type CallMessage = ();
/// #    type Event = ();
/// #
/// #    fn call(
/// #        &mut self,
/// #        _msg: Self::CallMessage,
/// #        _context: &Context<Self::Spec>,
/// #        _state: &mut impl sov_modules_api::state::TxState<S>,
/// #    ) -> anyhow::Result<()> {
/// #        unimplemented!()
/// #    }
/// # }
/// # // END MODULE IMPL
/// ```
///
/// ## Attributes: `#[rest_api(doc = "...")]`
///
/// Overrides the description of the marked item used in the generated
/// metadata. By default, descriptions are fetched from docstrings.
///
/// You can use this attribute at the top of the module as well as state items.
///
/// ```
/// use sov_modules_api::prelude::*;
/// use sov_modules_api::{ModuleId, ModuleInfo, StateMap};
///
/// /// This docstring will not be used.
/// #[derive(Clone, ModuleInfo, ModuleRestApi)]
/// #[rest_api(doc = "This is a description of the module.")]
/// #[rest_api(doc = "")]
/// #[rest_api(doc = "This is a second paragraph in the description.")]
/// struct MyModule<S: Spec> {
///     #[id]
///     id: ModuleId,
///     /// This description will not be used.
///     #[state]
///     #[rest_api(doc = "My favorite state item!")]
///     state_item: StateMap<S::Address, u64>,
/// }
/// # // BEGIN MODULE IMPL, COPY-PASTE-ME (https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#hiding-portions-of-the-example)
/// # impl<S: Spec> sov_modules_api::Module for MyModule<S> {
/// #    type Spec = S;
/// #    type Config = ();
/// #    type CallMessage = ();
/// #    type Event = ();
/// #
/// #    fn call(
/// #        &mut self,
/// #        _msg: Self::CallMessage,
/// #        _context: &Context<Self::Spec>,
/// #        _state: &mut impl sov_modules_api::state::TxState<S>,
/// #    ) -> anyhow::Result<()> {
/// #        unimplemented!()
/// #    }
/// # }
/// # // END MODULE IMPL
/// ```
pub use sov_modules_macros::ModuleRestApi;

/// Procedural macros to assist with creating new modules.
pub mod macros {
    /// Reads a TOML value from the rollup configuration manifest file and
    /// converts it into a Rust expression.
    ///
    /// ## The manifest file
    ///
    /// The manifest file must be named `constants.toml` and it's searched
    /// upwards starting from `OUT_DIR`. It's recommended to put it and the root
    /// of your Cargo workspace.
    ///
    /// ## The `[constants]` section
    ///
    /// Inside the `[constants]` section, you can define constants that will then be
    /// accessible in your code via [`config_value!`] macro.
    ///
    /// ```toml
    /// [constants]
    ///
    /// # assert!(config_value!("BOOL"));
    /// BOOL = true
    ///
    /// # assert_eq!(config_value!("UINT"), 42);
    /// UINT = 42
    ///
    /// # assert_eq!(config_value!("STRING"), "foo");
    /// STRING = "foo"
    ///
    /// # assert_eq!(config_value!("ARRAY_OF_U8"), [1, 2, 3]);
    /// ARRAY_OF_U8 = [1, 2, 3]
    ///
    /// # assert_eq!(config_value!("TOKEN_ID"), ...);
    /// TOKEN_ID = { bech32 = "token_1qwqr2h2e5g961t4f2m1qt3t3d7xx7r4jchjc9ey5pe1r5u8ers9ts", type = "TokenId" }
    /// ```
    ///
    /// ## Overriding constants
    ///
    /// When testing your code, it's often useful to override constants. You can do that by setting the
    /// `SOV_TEST_CONST_OVERRIDE_{CONSTANT_NAME}` env. variable inside your test.
    ///
    /// ## `const` expressions
    ///
    /// If you want to use a TOML constant inside a `const` expression, you can do this:
    ///
    /// ```toml
    /// [constants]
    /// MY_CONST = { const = "foobar" }
    /// ```
    ///
    /// Note that this will disable overriding for this constant.
    pub use sov_modules_macros::config_value;
    /// The macro exposes RPC endpoints from all modules in the runtime.
    /// It gets storage from the Context generic
    /// and utilizes output of [`#[rpc_gen]`] macro to generate RPC methods.
    ///
    /// It has limitations:
    ///   - First type generic attribute must have bound to [`Context`](crate::Context) trait
    ///   - All generic attributes must own the data, thus have bound `'static`
    #[cfg(feature = "native")]
    pub use sov_modules_macros::expose_rpc;
    /// A wrapper around [`jsonrpsee::proc_macros::rpc`] for modules.
    ///
    /// This proc-macro generates a [`jsonrpsee`] implementation for the underlying
    /// module type. It behaves very similar to the original [`jsonrpsee`]
    /// proc-macro, but with some important distinctions:
    ///
    /// 1. It's called `#[rpc_gen]` instead of `#[rpc]`, to avoid confusion with the
    ///    original proc-macro.
    /// 2. `#[method]` is renamed to with `#[rpc_method]` to avoid confusion with
    ///    [`jsonrpsee`]'s own attribute.
    /// 3. **It's applied on an `impl` block instead of a trait.** [`macro@rpc_gen`] will
    ///    copy all method definitions from your `impl` block into a new trait with
    ///    the same generics and method signatures. Unlike [`jsonrpsee`] traits
    ///    which can simply be signatures, these methods must all have function
    ///    bodies as they provide the trait definition and its implementation at the
    ///    same time.
    /// 4. Working set arguments with signature `state: &mut WorkingSet<S>`
    ///    are automatically removed from the method signatures (as they are not
    ///    [`serde`]-compatible) and injected directly within the method bodies.
    ///
    ///    It sounds more complicated than it is. Generally, you can just assume
    ///    that the proc-macro will provide you with a working set argument that you
    ///    can request by adding it as a method argument.
    ///
    /// Any code relying on this macro must take [`jsonrpsee`] as a dependency with
    /// at least the following features enabled:
    ///
    /// ```toml
    /// jsonrpsee = { version = "...", features = ["macros", "client-core", "server"] }
    /// ```
    ///
    /// This proc-macro is only intended for modules. Refer to [`macro@expose_rpc`] for
    /// the runtime proc-macro.
    ///
    /// ## Example
    /// ```
    /// use sov_modules_api::{Spec, StateValue, ModuleId, ModuleInfo, ApiStateAccessor, prelude::UnwrapInfallible};
    /// use sov_modules_api::macros::rpc_gen;
    /// use jsonrpsee::core::RpcResult;
    ///
    /// #[derive(ModuleInfo, Clone)]
    /// struct MyModule<S: Spec> {
    ///     #[id]
    ///     id: ModuleId,
    ///     #[state]
    ///     values: StateValue<S::Address>,
    ///     // ...
    /// }
    ///
    /// #[rpc_gen(client, server, namespace = "myNamespace")]
    /// impl<S: Spec> MyModule<S> {
    ///     #[rpc_method(name = "myMethod")]
    ///     fn my_method(&self, state: &mut ApiStateAccessor<S>, param: u32) -> RpcResult<S::Address> {
    ///         Ok(self.values.get(state).unwrap_infallible().unwrap())
    ///     }
    /// }
    /// ```
    #[cfg(feature = "native")]
    pub use sov_modules_macros::rpc_gen;
    /// A convenience attribute macro that adds serialization-related derives.
    /// It takes `Borsh` and/or `Serde` as arguments to generate the appropriate implementations.
    ///
    /// The macro generates the following for each argument:
    /// - `Borsh`: `#[derive(borsh::BorshDeserialize, borsh::BorshSerialize)]`
    /// - `Serde`: `#[derive(serde::Serialize, serde::Deserialize)]`
    ///
    /// ## Example
    /// ```rust
    /// use sov_modules_api::macros::serialize;
    ///
    /// #[serialize(Borsh, Serde)]
    /// #[derive(Debug, PartialEq)] // Other derives are still needed
    /// pub enum MyMessage {
    ///     One,
    ///     Two(u32),
    ///     Three { data: Vec<u8> }
    /// }
    /// ```
    pub use sov_modules_macros::serialize;
    /// Implements the `sov_modules_api::CliWallet` trait for the annotated runtime.
    /// Under the hood, this macro generates an enum called `CliTransactionParser` which derives the [`clap::Parser`] trait.
    /// This enum has one variant for each field of the `Runtime`, and uses the `sov_modules_api::CliWalletArg` trait to parse the
    /// arguments for each of these structs.
    ///
    /// To exclude a module from the CLI, use the `#[cli_skip]` attribute.
    ///
    /// ## Examples
    /// ```
    /// use sov_modules_api::{Spec, DispatchCall, MessageCodec};
    /// use sov_modules_api::macros::CliWallet;
    ///
    /// #[derive(DispatchCall, MessageCodec, CliWallet)]
    /// pub struct Runtime<S: Spec> {
    ///     pub bank: sov_bank::Bank<S>,
    ///     // ...
    /// }
    ///
    /// fn main() {}
    //  ^^^^^^^^^^^^
    //  COMMENT: the above `main` function is a workaround for
    //  <https://github.com/rust-lang/rust/issues/83583#issuecomment-1083300448>.
    /// ```
    #[cfg(feature = "native")]
    pub use sov_modules_macros::CliWallet;
    /// Derives [`HasRestApi`](crate::rest::HasRestApi) for runtimes.
    ///
    /// For each module listed in this runtime, the proc-macro will mount its
    /// own REST API at
    /// `/modules/<hyphenated-module-name>`. Consult the documentation of
    /// [`crate::rest`] for more information about these traits.
    ///
    /// Modules listed in the runtime for which no
    /// [`crate::ModuleRestApi`] is derived will simply be ignored.
    ///
    /// ## Attributes: `#[rest_api(skip)]`
    ///
    /// Tells the proc-macro to **NOT** generate a REST API for the marked module.
    ///
    /// ## Attributes: `#[rest_api(doc)]`
    ///
    /// This attribute behaves exactly the same as it does for
    /// [`crate::ModuleRestApi`].
    pub use sov_modules_macros::RuntimeRestApi;
    /// Implements the [`UniversalWallet`](sov_universal_wallet::schema::UniversalWallet) trait for the
    /// annotated struct or enum.
    ///
    /// The schema generated by the trait allows two main features.
    /// First, the borsh-encoding of the type to be dispalyed in a human-readable format, with the exact
    /// formatting controlled by attributes on the fields of the type.
    /// Second, it allows a JSON-encoding of the type to be translated into borsh-encoding, without
    /// needing access to the original Rust definition of the type.
    ///
    /// ## Attributes: `#[sov_wallet(bound = "T: Trait")]`
    ///
    /// Tells the proc-macro to add the specified bound to the where clause
    /// of the generated implementation instead of adding the default `T: UniversalWallet` (where `T`
    /// is the type of the annotated field).
    ///
    /// This annotation may only be applied to fields, not items.
    ///
    /// ## Attributes: `#[sov_wallet(hidden)]`
    ///
    /// Causes the field to be hidden from the user during display. This is often used for data
    /// that can't be displayed in a human-readable format, such as merkle proofs. If the field is not
    /// present in the `borsh` serialization of the type, use `#[sov_wallet(skip)]` instead.
    ///
    /// This annotation may only be applied to fields, not items.
    ///
    /// ```rust
    /// use sov_universal_wallet::schema::Schema;
    /// use sov_modules_api::macros::UniversalWallet;
    /// use sov_modules_api::SafeString;
    ///
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct Unreadable {
    ///    name: SafeString,
    ///    #[sov_wallet(hidden)]
    ///    opaque_contents: Vec<u8>,
    /// }
    /// let serialized = borsh::to_vec(&Unreadable { name: "foo.txt".try_into().unwrap(), opaque_contents: vec![23, 74, 119, 119, 2, 232, 22]}).unwrap();
    /// assert_eq!(Schema::of_single_type::<Unreadable>().unwrap().display(0, &serialized).unwrap(), r#"{ name: "foo.txt" }"#);
    /// ```
    /// Notice also the use of the `SafeString` type here - this is to ensure the string can be safely
    /// displayed to the user. By default, unconstrained Strings are forbidden in schemas; for blobs of
    /// data, use byte arrays/vectors directly. If a String is absolutely required, a newtype wrapper
    /// can be used.
    ///
    /// ## Attributes: `#[sov_wallet(as_ty = "path::to::Type")]`
    ///
    /// Inserts the schema of the specified type in place of the schema for the annotated field. Note that the subsituted type
    /// must have exactly the same borsh serialization as the original.
    ///
    /// This is useful when you want to display a foreign type that doesn't implement [`UniversalWallet`](sov_universal_wallet::schema::UniversalWallet),
    /// or when you want to override the default schema for a type in a particular context.
    ///
    /// ```rust
    /// use sov_rollup_interface::sov_universal_wallet::{schema::Schema, UniversalWallet};
    ///
    /// // A foreign type that doesn't derive UniversalWallet
    /// #[derive(borsh::BorshSerialize)]
    /// pub struct Foreign(u64);
    ///
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct Tagged {
    ///    #[sov_wallet(as_ty = "u64")]
    ///    data: Foreign,
    ///    tag: i8,
    /// }
    /// let serialized = borsh::to_vec(&Tagged { data: Foreign(300_000), tag: -5 }).unwrap();
    /// assert_eq!(Schema::of_single_type::<Tagged>().unwrap().display(0, &serialized).unwrap(), r#"{ data: 300000, tag: -5 }"#);
    /// ```
    ///
    /// ## Attributes: `#[sov_wallet(fixed_point({decimals}))]`
    ///
    /// Specifies fixed-point formatting for an integer field. The decimals can be specified in one
    /// of the following ways:
    ///  - `fixed_point(n)` where `n` is an integer literal, e.g. `fixed_point(18)`
    ///  - `fixed_point(from_field({n}))` or `fixed_point(from_field(n, offset=m))` where `n` and
    ///    `m` are integer literals: this causes the formatting to refer to the `nth` field within
    ///    the same parent structure (struct or tuple), by index, and read a single byte at the
    ///    given offset `m`. The offset defaults to `0` if not specified.
    ///
    /// ```rust
    /// use sov_rollup_interface::sov_universal_wallet::{schema::Schema, UniversalWallet};
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct Coins {
    ///     #[sov_wallet(fixed_point(from_field(1)))]
    ///     amount: u128,
    ///     #[sov_wallet(hidden)]
    ///     decimals: u8
    /// }
    /// let serialized = borsh::to_vec(&Coins { amount: 475200, decimals: 3 }).unwrap();
    /// assert_eq!(Schema::of_single_type::<Coins>().unwrap().display(0, &serialized).unwrap(), r#"{ amount: 475.2 }"#);
    /// ```
    ///
    /// **Security note**: uniquely, this formats the display using user-submitted input. If the
    /// accuracy of the displayed string is important for security, it is crucial that the submitted
    /// value for the amount of decimals be treated as the source of truth, as that will be what the
    /// user will have been presented with.
    /// For example, when using the schema to sign on-chain messages referencing cryptocurrency
    /// amounts, any message where the decimals field does not match the currency's canonical decimal
    /// count **must** be considered invalid and rejected.
    ///
    /// ## Attributes: `#[sov_wallet(template({template spec}))]`
    ///
    /// Annotates the field for inclusion in a standard template.
    ///
    /// Templates specify several input bindings that can be later provided to the schema (by
    /// name), and the schema will fill the inputs with the correct encoding to output a fully
    /// encoded target type. In this context, the target can be any of the schema root types.
    ///
    /// Fields can be annotated with either an input binding or a pre-defined value (which will be
    /// hardcoded into the template).
    ///
    /// The contents of the template attribute are of the format
    /// ```ignore
    /// template("template_one" = {field data}, "template_two" = {metadata}, ...)
    /// ```
    /// Where templates are defined (and distinguished) by their string name. The field data, in
    /// turn, is one of either
    /// * `input` for an input binding on the field name,
    /// * `input("name")` for an input binding with an arbitrary name, or
    /// * `value("data")` for a pre-defined hardcoded value, or
    /// * `value(bytes = "data")` for byte fields, reusing the field's `sov_wallet(display)`
    ///   attribute for parsing; or
    /// * `value(default)` to use the type's `std::default::Default::default()` value in the
    ///   template.
    ///
    /// Note that input names must be unique throughout a single template. For example, it's not
    /// possible to annotate two identically-named fields (in different structs) with `input` and
    /// have them be part of the same template; and it is not possible to pass the same string
    /// twice as part of `input("name")` within the same template.
    ///
    /// If a type definition annotates template attributes on one of its fields, all of its fields
    /// must have template metadata. Complex types that have their own subfields can be annotated
    /// at a lower level, and a field of such a type will be considered correctly annotated for the
    /// template when used in parent types.
    ///
    /// **Enum annotation:**
    /// As a special rule, in an enum, a template can only be defined on the fields of a single
    /// variant. It is an error to have template attributes with the same name available from
    /// multiple variants.
    ///
    /// To prevent unexpected/undesired inheriting of templates causing an error as per above, enum
    /// variants must be annotated with the following syntax:
    /// ```ignore
    /// template("template_name", ...)
    /// ```
    /// I.e. specifying a list of names, with no extra data necessary. Only templates explicitly
    /// named in the variant's own attribute will be available on that attribute.
    ///
    /// ```rust
    /// use sov_universal_wallet::schema::Schema;
    /// use sov_universal_wallet::schema::safe_string::SafeString;
    /// use sov_modules_api::macros::UniversalWallet;
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub enum CallMessage {
    ///     #[sov_wallet(template("transfer"))]
    ///     Transfer {
    ///         #[sov_wallet(template("transfer" = input("to")))]
    ///         to: SafeString,
    ///         coins: Coins,
    ///     }
    /// }
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct Coins {
    ///     #[sov_wallet(template("transfer" = input("amount")))]
    ///     pub amount: u128,
    ///     #[sov_wallet(template("transfer" = value("MY_TOKEN_ID")))]
    ///     pub token_id: SafeString,
    /// }
    ///
    /// let schema = Schema::of_single_type::<CallMessage>().unwrap();
    /// let encoded_call = schema.fill_template_from_json(0, "transfer", r#"{ "to":
    /// "sov1234_whatever_address", "amount": 2000 }"#).unwrap();
    ///
    /// assert_eq!(schema.display(0, &encoded_call).unwrap(), r#"Transfer { to: "sov1234_whatever_address", coins: { amount: 2000, token_id: "MY_TOKEN_ID" } }"#);
    /// ```
    ///
    /// ## Attributes: `#[sov_wallet(template_inherit)]`
    /// Applies to enums only. _Overrides_ the default behaviour, described above, which normally
    /// makes templates opt-in for enum variants. When `template_inherit` is specified on an enum,
    /// instead, every enum variant will automatically inherit all templates available on the type
    /// of that variant.
    ///
    /// ## Attributes: `#[sov_wallet(template_override_ty = "RemoteType")]`
    ///
    /// Sets the named type to be the source of inherited template definitions, rather than the
    /// actual field's type. This is useful when importing crates, such as sov module
    /// implementations, that define their own `#[sov_wallet(template(...))]` attributes on their
    /// types that are not relevant for the rollup.
    ///
    /// * An easy way to disable any inherited templates from a field's type is to set
    ///   `#[sov_wallet(template_override_ty = "()")]`.
    ///
    /// * To actually replace the templates with your own, build a set of scaffold types that
    ///   mirror the structure of the original, provide the desired `#[sov_wallet(template(...))]`
    ///   attributes and #[derive(UniversalWallet)] on the type; then it can be used as the argument
    ///   to `template_override_ty`.
    ///
    /// ```rust
    /// use sov_rollup_interface::sov_universal_wallet::{schema::Schema, UniversalWallet};
    ///
    /// /// A foreign module
    /// mod foreign {
    ///     use sov_modules_api::macros::UniversalWallet;
    ///     #[derive(UniversalWallet, borsh::BorshSerialize)]
    ///     pub struct ForeignData {
    ///         #[sov_wallet(template("call" = input("hello_world")))]
    ///         data: u64,
    ///     }
    /// }
    ///
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct SurrogateDataStruct {
    ///     #[sov_wallet(template("call" = input("my_data")))]
    ///     data: u64,
    /// }
    ///
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct CallMessage {
    ///    #[sov_wallet(template_override_ty = "SurrogateDataStruct")]
    ///    data: foreign::ForeignData,
    ///    #[sov_wallet(template("call" = input("extra_data")))]
    ///    extra_data: u8,
    /// }
    ///
    /// let schema = Schema::of_single_type::<CallMessage>().unwrap();
    /// let encoded_call = schema.fill_template_from_json(
    ///     0,
    ///     "call",
    ///     r#"{ "my_data": 12, "extra_data": 8 }"#
    /// ).unwrap();
    ///
    /// assert_eq!(schema.display(0, &encoded_call).unwrap(), r#"{ data: { data: 12 }, extra_data: 8 }"#);
    ///
    /// // Without using template_override_ty, the template would've looked like this:
    /// // let encoded_call = schema.fill_template_from_json(
    /// //    0,
    /// //    "call",
    /// //    r#"{ "foreign_data": 12, "extra_data": 8 }"#
    /// // ).unwrap();
    ///
    /// ```
    ///
    /// ## Attributes: `#[sov_wallet(skip)]`
    ///
    /// Causes the field to be excluded from the Schema entirely. This should be used if the field is not present in
    /// the `borsh` serialization of the type. If the type is present in the serialization but should not be displayed,
    /// use `#[sov_wallet(hidden)]` instead.
    ///
    /// ```rust
    /// use sov_universal_wallet::schema::Schema;
    /// use sov_modules_api::macros::UniversalWallet;
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct File {
    ///     #[borsh(skip)]
    ///     #[sov_wallet(skip)]
    ///     checksum: Option<[u8;32]>,
    ///     contents: Vec<u8>,
    /// }
    /// let serialized = borsh::to_vec(&File { contents: vec![1, 2, 3], checksum: None }).unwrap();
    /// assert_eq!(Schema::of_single_type::<File>().unwrap().display(0, &serialized).unwrap(), r#"{ contents: 0x010203 }"#);
    /// ```
    ///
    /// /// ## Attributes: `#[sov_wallet(hide_tag)]`
    ///
    /// Causes the tag of an enum to be skipped when displaying from its human-readable representation.
    ///
    /// ```rust
    /// use sov_universal_wallet::schema::Schema;
    /// use sov_modules_api::macros::UniversalWallet;
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// #[sov_wallet(hide_tag)]
    /// pub enum Example {
    ///     Hash([u8;32]),
    ///     Value(u64)
    /// }
    /// let serialized = borsh::to_vec(&Example::Value(1)).unwrap();
    /// assert_eq!(Schema::of_single_type::<Example>().unwrap().display(0, &serialized).unwrap(), "1");
    /// ```
    ///
    /// ## Attributes: `#[sov_wallet(display({encoding}))]`
    ///
    /// Specifies the encoding to use when displaying a byte sequence. The encoding can be one of the following:
    /// - `hex`: displays the type as a hexadecimal string with the prefix "0x"
    /// - `decimal`: displays the type as a list of decimal numbers in square brackets
    /// - `bech32(prefix = "my_prefix_expr")`: displays the type as a bech32-encoded string with the specified human-readable part.
    /// - `bech32m(prefix = "my_prefix_expr")`: displays the type as a bech32m-encoded string with the specified human-readable part.
    ///
    /// This annotation may only be applied to fields, not items. The field must have type `[u8;N]` or `Vec<u8>` to use this attribute.
    ///
    /// ```rust
    /// use sov_universal_wallet::schema::Schema;
    /// use sov_modules_api::macros::UniversalWallet;
    ///
    /// fn prefix() -> &'static str {
    ///   "celestia"
    /// }
    ///
    /// #[derive(UniversalWallet, borsh::BorshSerialize)]
    /// pub struct CelestiaAddress(
    ///   #[sov_wallet(display(bech32(prefix = "prefix()")))]
    ///   [u8;32],
    /// );
    /// let serialized = borsh::to_vec(&CelestiaAddress([1; 32])).unwrap();
    /// assert_eq!(Schema::of_single_type::<CelestiaAddress>().unwrap().display(0, &serialized).unwrap(), "celestia1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqsagv2r7");
    /// ```
    pub use sov_modules_macros::UniversalWallet;
}
