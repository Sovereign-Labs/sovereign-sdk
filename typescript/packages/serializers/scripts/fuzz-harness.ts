// Ran by https://github.com/Sovereign-Labs/sovereign-sdk/blob/nightly/crates/universal-wallet/fuzz/fuzz_targets/fuzz_js_impl.rs
// as part of fuzz testing suite.
import { bytesToHex } from "@sovereign-sdk/utils";
import { JsSerializer } from "../src";

const schema = JSON.parse(process.argv[2]);
const serializer = new JsSerializer(schema);
const input = JSON.parse(process.argv[3]);
const result = serializer.serialize(input, 0);
console.log(bytesToHex(result));
