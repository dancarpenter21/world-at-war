use std::{env, path::PathBuf, process::ExitCode};

use sim_comms::CommunicationsCatalog;

fn main() -> ExitCode {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/communications/catalog.yaml"));
    match CommunicationsCatalog::load(&path) {
        Ok(catalog) => {
            println!("communications catalog: {}", path.display());
            println!("version: {} (as of {})", catalog.version, catalog.as_of);
            println!("catalog checksum: {}", catalog.checksum());
            println!("message-pack checksum: {}", catalog.message_pack_checksum());
            println!(
                "coverage: {} entity assignments, {} platforms, {} devices, {} radio modes",
                catalog.assignments.len(),
                catalog.platforms.len(),
                catalog.devices.len(),
                catalog
                    .devices
                    .iter()
                    .map(|device| device.radio_modes.len())
                    .sum::<usize>()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            ExitCode::FAILURE
        }
    }
}
