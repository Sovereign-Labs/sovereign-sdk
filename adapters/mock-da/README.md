# `sov-mock-da`

Mock implementation of `DaService`, `DaSpec` and `DaVerifier` traits.

Used for testing and demo purposes.


sov-mock-da should be imported with "native" flag if any module is imported with the native flag. 
Modules indirectly import rollup-interface with native,
which means that sov-mock-da cannot fully implement BlobReader if it also does not have "native".