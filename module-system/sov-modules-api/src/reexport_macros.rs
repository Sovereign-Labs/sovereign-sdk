/// Derives the [`DispatchCall`] trait for the underlying
/// type.
#[cfg(feature = "macros")]
pub use sov_modules_macros::DispatchCall;
/// Derives the [`Genesis`](trait.Genesis.html) trait for the underlying runtime
/// `struct`.
#[cfg(feature = "macros")]
pub use sov_modules_macros::Genesis;
#[cfg(feature = "macros")]
pub use sov_modules_macros::MessageCodec;
/// Derives the [`ModuleCallJsonSchema`](trait.ModuleCallJsonSchema.html) trait for
/// the underlying type.
///
/// ## Example
///
/// ```
/// use std::marker::PhantomData;
///
/// use sov_modules_api::{Context, Module, ModuleInfo, ModuleCallJsonSchema};
/// use sov_modules_api::default_context::ZkDefaultContext;
/// use sov_state::StateMap;
/// use sov_bank::CallMessage;
///
/// #[derive(ModuleInfo, ModuleCallJsonSchema)]
/// struct TestModule<C: Context> {
///     #[address]
///     admin: C::Address,
///
///     #[state]
///     pub state_map: StateMap<String, u32>,
/// }
///
/// impl<C: Context> Module for TestModule<C> {
///     type Context = C;
///     type Config = PhantomData<C>;
///     type CallMessage = CallMessage<C>;
/// }
///
/// println!("JSON Schema: {}", TestModule::<ZkDefaultContext>::json_schema());
/// ```
#[cfg(feature = "macros")]
pub use sov_modules_macros::ModuleCallJsonSchema;
/// Derives the [`ModuleInfo`] trait for the underlying `struct`.
///
/// The underlying type must respect the following conditions, or compilation
/// will fail:
/// - It must be a named `struct`. Tuple `struct`s, `enum`s, and others are
/// not supported.
/// - It must have *exactly one* field with the `#[address]` attribute. This field
///   represents the **module address**.
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
/// use sov_modules_api::{Context, ModuleInfo};
/// use sov_state::StateMap;
///
/// #[derive(ModuleInfo)]
/// struct TestModule<C: Context> {
///     #[address]
///     admin: C::Address,
///
///     #[state]
///     pub state_map: StateMap<String, u32>,
/// }
///
/// // You can then get the prefix of `state_map` like this:
/// fn get_prefix<C: Context>(some_storage: C::Storage) {
///     let test_struct = TestModule::<C>::default();
///     let prefix1 = test_struct.state_map.prefix();
/// }
/// ```
#[cfg(feature = "macros")]
pub use sov_modules_macros::ModuleInfo;

/// Procedural macros to assist with creating new modules.
#[cfg(feature = "macros")]
pub mod macros {
    /// The macro exposes RPC endpoints from all modules in the runtime.
    /// It gets storage from the Context generic
    /// and utilizes output of [`#[rpc_gen]`] macro to generate RPC methods.
    ///
    /// It has limitations:
    ///   - First type generic attribute must have bound to [`Context`](crate::Context) trait
    ///   - All generic attributes must own the data, thus have bound `'static`
    #[cfg(feature = "native")]
    pub use sov_modules_macros::expose_rpc;
    #[cfg(feature = "native")]
    pub use sov_modules_macros::rpc_gen;
    /// Implements the `sov_modules_api::CliWallet` trait for the annotated runtime.
    /// Under the hood, this macro generates an enum called `CliTransactionParser` which derives the [`clap::Parser`] trait.
    /// This enum has one variant for each field of the `Runtime`, and uses the `sov_modules_api::CliWalletArg` trait to parse the
    /// arguments for each of these structs.
    ///
    /// To exclude a module from the CLI, use the `#[cli_skip]` attribute.
    ///
    /// ## Examples
    /// ```
    /// use sov_modules_api::{Context, DispatchCall, MessageCodec};
    /// use sov_modules_api::default_context::DefaultContext;
    /// use sov_modules_api::macros::CliWallet;
    ///
    /// #[derive(DispatchCall, MessageCodec, CliWallet)]
    /// #[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
    /// pub struct Runtime<C: Context> {
    ///     pub bank: sov_bank::Bank<C>,
    ///     // ...
    /// }
    /// ```
    #[cfg(feature = "native")]
    pub use sov_modules_macros::CliWallet;
    /// Implement [`CliWalletArg`](crate::CliWalletArg) for the annotated struct or enum. Unions are not supported.
    ///
    /// Under the hood, this macro generates a new struct or enum which derives the [`clap::Parser`] trait, and then implements the
    /// [`CliWalletArg`](crate::CliWalletArg) trait where the `CliStringRepr` type is the new struct or enum.
    ///
    /// As an implementation detail, `clap` requires that all types have named fields - so this macro auto generates an appropriate
    /// `clap`-compatible type from the annotated item. For example, the struct `MyStruct(u64, u64)` would be transformed into
    /// `MyStructWithNamedFields { field0: u64, field1: u64 }`.
    ///
    /// ## Example
    ///
    /// This code..
    /// ```rust
    /// use sov_modules_api::macros::CliWalletArg;
    /// #[derive(CliWalletArg, Clone)]
    /// pub enum MyEnum {
    ///    /// A number
    ///    Number(u32),
    ///    /// A hash
    ///    Hash { hash: String },
    /// }
    /// ```
    ///
    /// ...expands into the following code:
    /// ```rust,ignore
    /// // The original enum definition is left in its original place
    /// pub enum MyEnum {
    ///    /// A number
    ///    Number(u32),
    ///    /// A hash
    ///    Hash { hash: String },
    /// }
    ///
    /// // We generate a new enum with named fields which can derive `clap::Parser`.
    /// // Since this variant is only ever converted back to the original, we
    /// // don't carry over any of the original derives. However, we do preserve
    /// // doc comments from the original version so that `clap` can display them.
    /// #[derive(::clap::Parser)]
    /// pub enum MyEnumWithNamedFields {
    ///    /// A number
    ///    Number { field0: u32 } ,
    ///    /// A hash
    ///    Hash { hash: String },
    /// }
    /// // We generate a `From` impl to convert between the types.
    /// impl From<MyEnumWithNamedFields> for MyEnum {
    ///    fn from(item: MyEnumWithNamedFields) -> Self {
    ///       match item {
    ///         Number { field0 } => MyEnum::Number(field0),
    ///         Hash { hash } => MyEnum::Hash { hash },
    ///       }
    ///    }
    /// }
    ///
    /// impl sov_modules_api::CliWalletArg for MyEnum {
    ///     type CliStringRepr = MyEnumWithNamedFields;
    /// }
    /// ```
    #[cfg(feature = "native")]
    pub use sov_modules_macros::CliWalletArg;
    /// Derives a custom [`Default`] implementation for the underlying type.
    /// We decided to implement a custom macro DefaultRuntime that would implement a custom Default
    /// trait for the Runtime because the stdlib implementation of the default trait imposes the generic
    /// arguments to have the Default trait, which is not needed in our case.
    pub use sov_modules_macros::DefaultRuntime;
}
