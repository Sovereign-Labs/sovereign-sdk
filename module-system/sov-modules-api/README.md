# `sov-modules-api`

The `sov-modules-api` crate provides essential traits for the Module System. Here are the key traits defined by the
crate:

1. The `Module` trait: Defines how to initialize and change the state of a module. This is the main trait that module
   developers need to implement. The author of a module must specify:

    - Configuration upon rollup deployment: This includes the `genesis()` method and the `Config` type, which determine
      how the module is set up initially. Note that the initialization for logic for modules is identical to
      the `Genesis` trait (described below). We blanket implement `Genesis`
      for all `Module`s, but keep it as a separate trait since some other structs need to implement it as well.

    - Interaction with user messages: The module must define the `call` method and the `CallMessage` type, which handle
      user messages. These messages typically result in changes to the module's state.

    - Gas configuration: The module may use a `GasConfig` type, annotated by `#[gas]`, that will be loaded from the 
      constants manifest configuration.

1. The `ModuleInfo` trait: Provides additional information related to a module. This trait is automatically derived.

1. The `Spec` trait: It defines all the types that modules are generic over. This separation allows the module logic to
   be independent of concerns such as the specific storage system or concrete signature schemes used for signing rollup
   transactions. Currently acceptable hashes for `Spec` should fit into 32 bytes.

1. The `Context` trait implements the `Spec` and introduces additional methods accessible within modules. Currently, it
   includes the `sender()` method, which returns the address of the transaction sender. This trait will be further
   extended with other useful methods, such as `batch_hash()`, and more. This crate defines also the default
   implementation for the `Context` trait.

1. The `Genesis` trait: Defines how the rollup is initialized during deployment phase.

1. The `DispatchCall` trait: Defines how messages are forwarded to the appropriate module and how the call message is
   executed. The implementation of this trait can be generated automatically using a macro.

1. The `GasUnit` trait: Defines how the scalar gas value is deducted from the working set. This is implemented for
   `[u64; N]`, and can be customized by the user.
