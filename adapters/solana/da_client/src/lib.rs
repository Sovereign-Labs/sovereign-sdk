use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

pub fn write_random_bytes<P: AsRef<Path>>(path: P, size: u64) -> std::io::Result<()> {
    let mut file = File::create(path)?;

    let random_bytes: Vec<u8> = (0..size).map(|_| rand::random::<u8>()).collect();

    file.write_all(&random_bytes)
}

pub fn read_file_to_vec<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();

    file.read_to_end(&mut buffer)?;

    Ok(buffer)
}
