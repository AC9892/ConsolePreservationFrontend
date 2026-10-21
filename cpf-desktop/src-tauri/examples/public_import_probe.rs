// Local release test harness. It is never included in the public ZIP.
#[path = "../src/public_setup.rs"]
mod public_setup;

use cpf_core::Workspace;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 8 { return Err("Usage: public_import_probe ROOT BIOS9 BIOS7 BIOS9I BIOS7I FIRMWARE NAND".into()); }
    let root = PathBuf::from(&args[1]);
    fs::create_dir_all(root.join("machines")).map_err(|e| e.to_string())?;
    let workspace = Workspace::open(root)?;
    let result = public_setup::import(&workspace, public_setup::ImportFiles {
        bios9: args[2].clone(), bios7: args[3].clone(), bios9i: args[4].clone(),
        bios7i: args[5].clone(), firmware: args[6].clone(), nand: args[7].clone(),
    })?;
    println!("configured={} message={}", result.configured, result.message);
    Ok(())
}
