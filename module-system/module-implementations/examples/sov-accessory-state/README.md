# The `sov-accessory-state` module

This module has no useful functionality, but it illustrates the ability to store data outside of the JMT, called "accessory state".

Accessory state does not contribute to the state root hash and can be written during zkVM execution, but reads are only allowed in a native execution context. Accessory state data can be used for tooling, serving JSON-RPC data, debugging, and any other purpose that is not core to the core functioning of the module without incurring in major performance penalties.
