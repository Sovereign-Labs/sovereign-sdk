use std::path::Path;

const SPEC_PATH: &str = "./openapi-v3.yaml";

fn main() -> std::io::Result<()> {
    assert!(Path::new(SPEC_PATH).try_exists()?);

    println!("cargo::rerun-if-changed={SPEC_PATH}");
    Ok(())
}
