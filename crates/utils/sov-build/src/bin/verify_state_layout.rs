use std::fs::File;
use std::io::BufReader;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct StateLayout {
    modules: Vec<ModuleLayout>,
}

#[derive(Deserialize)]
struct ModuleLayout {
    name: String,
    discriminant: u8,
    #[serde(default)]
    state_items: Vec<StateItemLayout>,
}

#[derive(Deserialize)]
struct StateItemLayout {
    name: String,
    discriminant: u8,
    #[serde(default)]
    type_ident: String,
}

fn describe_module(module: &ModuleLayout) -> String {
    format!("{} (discriminant {})", module.name, module.discriminant)
}

fn compare_state_items(old_module: &ModuleLayout, new_module: &ModuleLayout) -> Result<()> {
    if old_module.state_items.len() > new_module.state_items.len() {
        bail!(
            "module {} lost state items (old {} > new {})",
            describe_module(old_module),
            old_module.state_items.len(),
            new_module.state_items.len()
        );
    }

    for (idx, old_item) in old_module.state_items.iter().enumerate() {
        let new_item = &new_module.state_items[idx];
        if old_item.name != new_item.name
            || old_item.discriminant != new_item.discriminant
            || old_item.type_ident != new_item.type_ident
        {
            bail!(
                "module {} state item changed at index {}: old {}={} ({}) vs new {}={} ({})",
                describe_module(old_module),
                idx,
                old_item.name,
                old_item.discriminant,
                old_item.type_ident,
                new_item.name,
                new_item.discriminant,
                new_item.type_ident
            );
        }
    }

    Ok(())
}

fn compare_modules(old_layout: &StateLayout, new_layout: &StateLayout) -> Result<()> {
    if old_layout.modules.len() > new_layout.modules.len() {
        bail!(
            "module list shrunk (old {} > new {})",
            old_layout.modules.len(),
            new_layout.modules.len()
        );
    }

    for (idx, old_module) in old_layout.modules.iter().enumerate() {
        let new_module = &new_layout.modules[idx];
        if old_module.name != new_module.name
            || old_module.discriminant != new_module.discriminant
        {
            bail!(
                "module changed at index {}: old {} vs new {}",
                idx,
                describe_module(old_module),
                describe_module(new_module)
            );
        }

        compare_state_items(old_module, new_module)?;
    }

    Ok(())
}

fn load_layout(path: &str) -> Result<StateLayout> {
    let file = File::open(path).with_context(|| format!("missing file: {path}"))?;
    let reader = BufReader::new(file);
    let layout = serde_json::from_reader(reader)
        .with_context(|| format!("invalid json in {path}"))?;
    Ok(layout)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        bail!("usage: verify_state_layout <old_layout.json> <new_layout.json>");
    }

    let old_layout = load_layout(&args[1])?;
    let new_layout = load_layout(&args[2])?;

    compare_modules(&old_layout, &new_layout)?;
    println!("state layout check passed");
    Ok(())
}
