use std::collections::HashMap;

fn main() {
    #[cfg(not(feature = "bench"))]
    let guest_pkg_to_options = HashMap::new();
    #[cfg(feature = "bench")]
    let mut guest_pkg_to_options = HashMap::new();
    #[cfg(feature = "bench")]
    guest_pkg_to_options.insert(
        "sov-demo-prover-guest",
        risc0_build::GuestOptions {
            features: vec!["bench".to_string()],
            std: true,
        },
    );
    risc0_build::embed_methods_with_options(guest_pkg_to_options);
}
