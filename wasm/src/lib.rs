//! WebAssembly bindings for PDB Inspector
//!
//! This crate provides JavaScript-callable functions for analyzing PDB files
//! directly in the browser.

use pdbinspector::{analyze_pdb_from_bytes, format_report, OutputFormat};
use wasm_bindgen::prelude::*;

/// Analyze a PDB file and return the report as a formatted string.
///
/// # Arguments
/// * `data` - The raw bytes of the PDB file
/// * `format` - Output format: "text", "json", or "grep"
/// * `detailed` - If true, include detailed lists of types and symbols
///
/// # Returns
/// A formatted string representation of the PDB analysis
#[wasm_bindgen]
pub fn analyze_pdb(data: &[u8], format: &str, detailed: bool) -> Result<String, JsValue> {
    let output_format: OutputFormat = format.parse().map_err(|e: String| JsValue::from_str(&e))?;

    let report = analyze_pdb_from_bytes(data, detailed)
        .map_err(|e| JsValue::from_str(&format!("PDB analysis failed: {}", e)))?;

    Ok(format_report(&report, output_format, true))
}

/// Get the version of the PDB Inspector library.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
