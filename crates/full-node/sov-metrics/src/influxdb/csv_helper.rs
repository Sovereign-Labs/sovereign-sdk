use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process;

use chrono::Utc;

/// Allows writing metrics to CSV file for debugging.
pub struct CsvWriters {
    pub(crate) constant_writer: BufWriter<File>,
    pub(crate) zk_vm_writer: BufWriter<File>,
}

fn create_writer(dir: &Path, file_name: &str, header: &str) -> io::Result<BufWriter<File>> {
    fs::create_dir_all(dir)?;
    let file_path = Path::new(dir).join(file_name);
    let mut writer = BufWriter::new(File::create(file_path)?);
    writer.write_all(header.as_bytes())?;
    Ok(writer)
}

impl CsvWriters {
    pub(crate) async fn new() -> io::Result<Self> {
        let process_id = process::id();
        let now = Utc::now().date_naive();

        let path = Path::new("data")
            .join(format!("{now}"))
            .join(format!("{process_id}"));

        let constant_writer = create_writer(
            &path,
            "constants_output.csv",
            "name,constant,num_invocations,pre_state_root\n",
        )?;

        let zk_vm_writer = create_writer(
            &path,
            "zk_vm.csv",
            "name,cycles_count,memory_used,free_heap_bytes,pre_state_root\n",
        )?;

        Ok(Self {
            constant_writer,
            zk_vm_writer,
        })
    }
}
