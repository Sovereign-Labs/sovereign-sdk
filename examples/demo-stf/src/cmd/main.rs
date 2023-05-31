use clap::Parser;
use std::fs;
use std::path::{Path};

use demo_stf::runtime::cmd_parser;

#[derive(Parser)]
enum Cli {
    SerializeCall {
        module_name: String,
        call_data_path: String,
    },
}

// cargo run --bin cmd -- serialize-call Bank src/bank_cmd/test_data/create_token.json

pub fn main() {
    let cli = Cli::parse();

    match cli {
        Cli::SerializeCall {
            module_name,
            call_data_path,
        } => {
            if !Path::new(&call_data_path).exists() {
                println!("File does not exist");
                return;
            }

            match fs::read_to_string(&call_data_path) {
                Ok(file_content) => {
                    let bytes = cmd_parser(&module_name, &file_content);
                    println!("{:?}", bytes.unwrap());
                }
                Err(e) => println!("There was an error reading the file: {}", e),
            };
        }
    };
}
