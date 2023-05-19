# sov-modules-api

The API provides set of traits used by the sov module system. 

1. The `Module` trait defines how initialize, change and query the state of a module. This is the main trait that has to be implemented by a module developer. 
1. The `ModuleInfo` trait provides additional information related to a module. This trait is auto derived.
1. The `Spec` trait defines all the types the modules are generic over. This decuples module logic from concerns like specific storage system or concrete signature schemes used for signing rollup transactions.
1. The `Context` traits implements `Spec` and defines additional methods that are avaliable inside modules (currently only sender of the transaction but other methods like current batch number etc can be added )

This carte defines also the default implementation for the `Context` trait.


