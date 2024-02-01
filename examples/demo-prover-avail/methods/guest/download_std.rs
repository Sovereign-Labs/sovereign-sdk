// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{
    default::Default,
    fs::{self, File},
    path::{Path, PathBuf},
};

use downloader::{Download, Downloader};
// use sha2::{Digest as ShaDigest, Sha256};
use tempfile::tempdir_in;
use zip::ZipArchive;

#[derive(Debug)]
struct ZipMapEntry {
    filename: &'static str,
    zip_url: &'static str,
    src_prefix: &'static str,
    dst_prefix: &'static str,
}

// Sources for standard library, and where they should be mapped to.
const RUST_LIB_MAP : &[ZipMapEntry] = &[
    ZipMapEntry {
        filename: "53bbc8fc2afb2e10e3a90d7bf188bfd6598374ab.zip",
        zip_url: "https://github.com/risc0/rust/archive/53bbc8fc2afb2e10e3a90d7bf188bfd6598374ab.zip",
        src_prefix: "rust-53bbc8fc2afb2e10e3a90d7bf188bfd6598374ab/library",
        dst_prefix: "library"
    },
    ZipMapEntry {
        filename: "790411f93c4b5eada3c23abb4c9a063fb0b24d99.zip",
        zip_url: "https://github.com/rust-lang/stdarch/archive/790411f93c4b5eada3c23abb4c9a063fb0b24d99.zip",
        src_prefix:"stdarch-790411f93c4b5eada3c23abb4c9a063fb0b24d99",
        dst_prefix: "library/stdarch"
    },
    ZipMapEntry {
        filename: "07872f28cd8a65c3c7428811548dc85f1f2fb05b.zip",
        zip_url: "https://github.com/rust-lang/backtrace-rs/archive/07872f28cd8a65c3c7428811548dc85f1f2fb05b.zip",
        src_prefix:"backtrace-rs-07872f28cd8a65c3c7428811548dc85f1f2fb05b",
        dst_prefix: "library/backtrace"
    },
];

fn setup_guest_build_env<P>(out_dir: P)
where
    P: AsRef<Path>,
{
    // Rust standard library.  If any of the RUST_LIB_MAP changed, we
    // want to have a different hash so that we make sure we recompile.
    // let (_, _) = sha_digest_with_hex(format!("{:?}", RUST_LIB_MAP).as_bytes());
    // TODO: This breaks change detection for the std source. Fix it
    let rust_lib_path = out_dir.as_ref().join("rust-std");
    if !rust_lib_path.exists() {
        println!(
            "Standard library {} does not exist; downloading",
            rust_lib_path.display()
        );

        download_zip_map(RUST_LIB_MAP, &rust_lib_path);
    }
}

fn risc0_cache() -> PathBuf {
    directories::ProjectDirs::from("com.risczero", "RISC Zero", "risc0")
        .unwrap()
        .cache_dir()
        .into()
}

fn download_zip_map<P>(zip_map: &[ZipMapEntry], dest_base: P)
where
    P: AsRef<Path>,
{
    let cache_dir = risc0_cache();
    if !cache_dir.is_dir() {
        fs::create_dir_all(&cache_dir).unwrap();
    }

    let temp_dir = tempdir_in(&cache_dir).unwrap();
    let mut downloader = Downloader::builder()
        .download_folder(temp_dir.path())
        .build()
        .unwrap();

    let tmp_dest_base = dest_base.as_ref().with_extension("downloadtmp");
    if tmp_dest_base.exists() {
        fs::remove_dir_all(&tmp_dest_base).unwrap();
    }

    for zm in zip_map.iter() {
        let src_prefix = Path::new(&zm.src_prefix);
        let dst_prefix = tmp_dest_base.join(&zm.dst_prefix);
        fs::create_dir_all(&dst_prefix).unwrap();

        let zip_path = cache_dir.join(zm.filename);
        if !zip_path.is_file() {
            println!(
                "Downloading {}, mapping {} to {}",
                zm.zip_url,
                zm.src_prefix,
                dst_prefix.display()
            );
            let dl = Download::new(zm.zip_url);
            downloader.download(&[dl]).unwrap().iter().for_each(|x| {
                let summary = x.as_ref().unwrap();
                println!("Downloaded: {}", summary.file_name.display());
            });
            fs::rename(temp_dir.path().join(zm.filename), &zip_path).unwrap();
        }

        let zip_file = File::open(zip_path).unwrap();
        let mut zip = ZipArchive::new(zip_file).unwrap();
        println!("Got zip with {} files", zip.len());

        let mut nwrote: u32 = 0;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).unwrap();
            let name = f.enclosed_name().unwrap();
            if let Ok(relative_src) = name.strip_prefix(src_prefix) {
                let dest_name = dst_prefix.join(relative_src);
                if f.is_dir() {
                    fs::create_dir_all(dest_name).unwrap();
                    continue;
                }
                if !f.is_file() {
                    continue;
                }
                std::io::copy(&mut f, &mut File::create(&dest_name).unwrap()).unwrap();
                nwrote += 1;
            }
        }
        println!("Wrote {} files", nwrote);
    }
    fs::rename(&tmp_dest_base, dest_base.as_ref()).unwrap();
}

/// Options defining how to embed a guest package in
/// [`embed_methods_with_options`].
pub struct GuestOptions {
    /// Features for cargo to build the guest with.
    pub features: Vec<String>,

    /// Enable standard library support
    pub std: bool,
}

impl Default for GuestOptions {
    fn default() -> Self {
        GuestOptions {
            features: vec![],
            std: true,
        }
    }
}

/// Embeds methods built for RISC-V for use by host-side dependencies.
/// Specify custom options for a guest package by defining its [GuestOptions].
/// See [embed_methods].
fn main() {
    let guest_dir = Path::new("riscv-guest-shim");

    setup_guest_build_env(&guest_dir);
}
