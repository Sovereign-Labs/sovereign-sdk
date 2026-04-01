use std::process::Command;

fn try_set_commit_hash_env() {
    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=GIT_COMMIT_HASH={hash}");
        }
    }
}

fn main() {
    try_set_commit_hash_env();
}
