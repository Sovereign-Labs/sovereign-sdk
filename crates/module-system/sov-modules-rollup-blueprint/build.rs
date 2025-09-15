fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()?;

    if output.status.success() {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!("cargo:rustc-env=GIT_COMMIT_HASH={}", hash);
    } else {
        println!("cargo:rustc-env=GIT_COMMIT_HASH=unknown");
    }

    Ok(())
}