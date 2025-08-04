#![no_main]
// A function inside the `fuzz_target` macro raises this warning
#![allow(non_snake_case)]

use std::process::Command;

use libfuzzer_sys::fuzz_target;
use sov_universal_wallet::schema::Schema;
use universal_wallet_fuzz::FuzzInput;

fuzz_target!(|input: FuzzInput| {
    let schema = Schema::of_single_type::<FuzzInput>().unwrap();
    let schema_json = serde_json::to_string(&schema).expect("failed to serialize schema");
    let input_json = serde_json::to_string(&input).expect("failed to serialize input");

    let js_dir =
        std::env::var("SOV_UNIVERSAL_WALLET_FUZZ_JS_DIR").unwrap_or_else(|_| "./js".to_string());

    let js_output = Command::new("bun")
        .arg("scripts/fuzz-harness.ts")
        .arg(schema_json)
        .arg(input_json)
        .current_dir(js_dir)
        .output()
        .expect("Failed to execute JS");

    if !js_output.status.success() {
        println!("JS stdout: {}", String::from_utf8_lossy(&js_output.stdout));
        println!("JS stderr: {}", String::from_utf8_lossy(&js_output.stderr));
        panic!(
            "JS process failed with exit code: {:?}",
            js_output.status.code()
        );
    }

    let js_result = String::from_utf8_lossy(&js_output.stdout)
        .trim()
        .to_string();
    let rust_serialized = borsh::to_vec(&input).expect("rust failed to serialize input");
    let rust_result = hex::encode(rust_serialized);

    assert_eq!(rust_result, js_result);
});
