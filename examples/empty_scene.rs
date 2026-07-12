//! Example: produce a minimal `.beui` file using the protocol crate.
//!
//! Run with: `cargo run -p beui-protocol --example empty_scene`
//!
//! Writes `target/empty_scene.beui` and prints it. Demonstrates the
//! two-line API: build an asset, save it.

use beui_protocol::{empty_asset, save_to_file, save_to_string};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let asset = empty_asset();

    // To a string (for inspection).
    let ron_text = save_to_string(&asset)?;
    println!("--- generated .beui ---");
    println!("{ron_text}");
    println!("--- end ---");

    // To a file.
    let out_path = std::path::Path::new("target/empty_scene.beui");
    save_to_file(&asset, out_path)?;
    println!("wrote {}", out_path.display());

    Ok(())
}