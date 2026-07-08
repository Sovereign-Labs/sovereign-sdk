---
"@sovereign-sdk/universal-wallet-wasm": minor
"@sovereign-sdk/serializers": minor
---

Update to sov-universal-wallet 0.4.1, which adds the `FromSiblingFieldWithOverride` fixed-point display variant. Rebuilds the wasm bindings so schemas using the new variant can be parsed, and adds the variant to the `FixedPointDisplay` type in the serializers package.
