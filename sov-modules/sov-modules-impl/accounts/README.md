# The Accounts module.


The module `call` messages are signed with a user private key, and include the corresponding public key for verification purposes, the public key pseudo anonymously identify the sender of the message. In principle we could use public keys for identity management in the module system




Bullets:
- A users signs a message with private key and the message contains corresponding public key for verification
- In the module system it is good to have one more level of abstraction. For example the the Bank module works with Addresses rathe than public key. This allows some flexibility for example a user can change an address fot its public key. And we can have addresses for which we don't know a public key (burn address)
