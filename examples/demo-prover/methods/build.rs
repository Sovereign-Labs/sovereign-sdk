use std::collections::HashMap;

fn main() {
    let guest_pkg_to_options = get_guest_options();
    risc0_build::embed_methods_with_options(guest_pkg_to_options);
}

#[cfg(not(feature = "bench"))]
fn get_guest_options() -> HashMap<&'static str, risc0_build::GuestOptions> {
    HashMap::new()
}

#[cfg(feature = "bench")]
fn get_guest_options() -> HashMap<&'static str, risc0_build::GuestOptions> {
    let mut guest_pkg_to_options = HashMap::new();
    guest_pkg_to_options.insert(
        "sov-demo-prover-guest",
        risc0_build::GuestOptions {
            features: vec!["bench".to_string()],
        },
    );
    guest_pkg_to_options
}
