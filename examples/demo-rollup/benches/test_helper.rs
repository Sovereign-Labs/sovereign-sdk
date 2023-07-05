use std::fs;
use std::io;

pub fn remove_dir_if_exists<P: AsRef<std::path::Path>>(path: P) -> io::Result<()> {
    if path.as_ref().exists() {
        fs::remove_dir_all(&path)
    } else {
        Ok(())
    }
}
