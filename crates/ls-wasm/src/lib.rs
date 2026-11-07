//! Thin wasm-bindgen wrapper over `ls-core`. The browser runs exactly the rules the contract
//! re-runs on settlement; this file only converts between JSON strings and the crate's types.

use wasm_bindgen::prelude::*;

/// Hash of the rules source this build was compiled from; compare with the contract's `core_version`.
#[wasm_bindgen]
pub fn rules_hash() -> String {
    ls_core::RULES_HASH.to_string()
}

/// Crate version string.
#[wasm_bindgen]
pub fn core_version() -> String {
    ls_core::VERSION.to_string()
}

fn err<E: core::fmt::Display>(e: E) -> JsError {
    JsError::new(&e.to_string())
}

/// Re-runs a whole dive (`entry_json` is a `DiveEntry`) and returns the `DiveOutcome` as JSON.
#[wasm_bindgen]
pub fn run_dive(seed: u64, entry_json: &str, moves: &[u8]) -> Result<String, JsError> {
    let entry: ls_core::DiveEntry = serde_json::from_str(entry_json).map_err(err)?;
    let outcome = ls_core::run_dive(seed, &entry, moves);
    serde_json::to_string(&outcome).map_err(err)
}
