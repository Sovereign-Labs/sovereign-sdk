use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process;

use clap::{Parser, Subcommand};
use toml_edit::{Array, DocumentMut, Item, Value};

/// Command-line interface for managing `[workspace.default-members]` in `Cargo.toml`
#[derive(Parser)]
#[command(
    name = "script",
    author,
    version,
    about = "Manage workspace.default-members via presets"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Replace all default members with the ones in the given preset
    Set {
        /// Name of the preset to load
        preset_name: String,
    },
    /// Save the current default members to a new preset file
    Save { preset_name: String },
    /// Add members from the given preset without replacing existing ones
    Add {
        /// Name of the preset to load
        preset_name: String,
    },
    /// Remove the `default-members` entry entirely
    Disable,
    /// Remove crates listed in the preset from the current default members
    Rm {
        /// Name of the preset to load
        preset_name: String,
    },
    /// List all available preset names
    Presets,
}

fn main() {
    let cli = Cli::parse();
    let cargo_toml_path = Path::new("Cargo.toml");

    if !cargo_toml_path.exists() {
        eprintln!("Error: `Cargo.toml` not found in the current directory.");
        process::exit(1);
    }

    match cli.command {
        Commands::Set { preset_name } => {
            set_default_members(cargo_toml_path, &preset_name);
        }
        Commands::Save { preset_name } => {
            save_preset(cargo_toml_path, &preset_name);
        }
        Commands::Add { preset_name } => {
            add_default_members(cargo_toml_path, &preset_name);
        }
        Commands::Disable => {
            disable_default_members(cargo_toml_path);
        }
        Commands::Rm { preset_name } => {
            remove_preset(cargo_toml_path, &preset_name);
        }
        Commands::Presets => {
            list_presets();
        }
    }
}

fn set_default_members(cargo_toml_path: &Path, preset_name: &str) {
    let preset = read_preset_members(preset_name);
    let mut doc = parse_cargo_toml(cargo_toml_path);

    let table = doc
        .get_mut("workspace")
        .unwrap()
        .as_table_mut()
        .expect("`workspace` should be a table");

    // Remove any existing default-members array
    table.remove_entry("default-members");

    // Create a new default-members array
    let default_members = table
        .entry("default-members")
        .or_insert(Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .unwrap();

    // Populate it with items from the preset
    for member in preset {
        default_members.push(member);
    }

    write_cargo_toml(cargo_toml_path, &doc);
}

fn save_preset(cargo_toml_path: &Path, preset_name: &str) {
    let mut doc = parse_cargo_toml(cargo_toml_path);
    let table = doc
        .get_mut("workspace")
        .unwrap()
        .as_table_mut()
        .expect("`workspace` should be a table");

    let default_members = table
        .entry("default-members")
        .or_insert(Item::Value(Value::Array(Array::new())))
        .as_array()
        .unwrap();

    // Gather the existing members
    let preset: Vec<String> = default_members
        .iter()
        .filter_map(|m| m.as_str().map(ToOwned::to_owned))
        .collect();

    let preset_path = Path::new("scripts")
        .join("switcheroo")
        .join("presets")
        .join(format!("{preset_name}.json"));

    let preset_file =
        File::create(&preset_path).unwrap_or_else(|_| panic!("Failed to create {preset_path:?}"));
    serde_json::to_writer_pretty(preset_file, &preset).expect("Failed to write preset JSON");
}

fn add_default_members(cargo_toml_path: &Path, preset_name: &str) {
    let preset = read_preset_members(preset_name);
    let mut doc = parse_cargo_toml(cargo_toml_path);

    let table = doc
        .get_mut("workspace")
        .unwrap()
        .as_table_mut()
        .expect("`workspace` should be a table");

    let default_members = table
        .entry("default-members")
        .or_insert(Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .unwrap();

    // Insert unique items
    for member in preset {
        if !default_members.iter().any(|m| m.as_str() == Some(&member)) {
            default_members.push(member);
        }
    }

    write_cargo_toml(cargo_toml_path, &doc);
    println!("Added crates from preset '{preset_name}' to [workspace.default-members].");
}

fn disable_default_members(cargo_toml_path: &Path) {
    let mut doc = parse_cargo_toml(cargo_toml_path);

    let table = doc
        .get_mut("workspace")
        .unwrap()
        .as_table_mut()
        .expect("`workspace` should be a table");

    if table.remove_entry("default-members").is_some() {
        write_cargo_toml(cargo_toml_path, &doc);
    }
}

fn remove_preset(cargo_toml_path: &Path, preset_name: &str) {
    let preset = read_preset_members(preset_name);
    let mut doc = parse_cargo_toml(cargo_toml_path);

    let table = doc
        .get_mut("workspace")
        .unwrap()
        .as_table_mut()
        .expect("`workspace` should be a table");

    if let Some(default_members) = table
        .get_mut("default-members")
        .and_then(Item::as_array_mut)
    {
        // Retain only members not listed in the preset
        default_members.retain(|m| {
            if let Some(member_str) = m.as_str() {
                !preset.contains(&member_str.to_string())
            } else {
                true
            }
        });

        write_cargo_toml(cargo_toml_path, &doc);
        println!("Removed crates from preset '{preset_name}' in [workspace.default-members].");
    } else {
        println!("No [workspace.default-members] section found, nothing to remove.");
    }
}

fn read_preset_members(preset_name: &str) -> Vec<String> {
    let preset_path = Path::new("scripts")
        .join("switcheroo")
        .join("presets")
        .join(format!("{preset_name}.json"));

    if !preset_path.exists() {
        eprintln!("Error: Preset '{preset_name}' not found at path {preset_path:?}");
        process::exit(1);
    }

    let preset_file = File::open(&preset_path)
        .unwrap_or_else(|_| panic!("Failed to open preset file {preset_path:?}"));

    serde_json::from_reader(preset_file).expect("Failed to parse preset JSON")
}

fn parse_cargo_toml(cargo_toml_path: &Path) -> DocumentMut {
    let cargo_toml_content = fs::read_to_string(cargo_toml_path)
        .unwrap_or_else(|_| panic!("Failed to read {cargo_toml_path:?}"));

    cargo_toml_content
        .parse::<DocumentMut>()
        .expect("Failed to parse Cargo.toml")
}

fn write_cargo_toml(cargo_toml_path: &Path, doc: &DocumentMut) {
    let mut file = File::create(cargo_toml_path)
        .unwrap_or_else(|_| panic!("Failed to create {cargo_toml_path:?}"));
    file.write_all(doc.to_string().as_bytes())
        .expect("Failed to write Cargo.toml");
}

fn list_presets() {
    let preset_dir = Path::new("scripts").join("switcheroo").join("presets");

    if !preset_dir.exists() {
        eprintln!("No presets directory found at path: {preset_dir:?}");
        return;
    }

    let Ok(entries) = fs::read_dir(&preset_dir) else {
        eprintln!("Failed to read directory: {preset_dir:?}");
        return;
    };

    // Print preset file names without `.json` extension
    println!("Available presets:");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                println!(" - {stem}");
            }
        }
    }
}
