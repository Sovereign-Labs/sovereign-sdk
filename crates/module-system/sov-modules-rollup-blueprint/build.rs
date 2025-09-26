fn main() {
    let hash = match std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => "unknown".to_string(),
    };

    println!("cargo:rustc-env=GIT_COMMIT_HASH={hash}");
}
