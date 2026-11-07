//! Thin wasm-bindgen wrapper over `ls-core`. The browser runs exactly the rules the contract
//! re-runs on settlement; this file only converts between JSON strings and the crate's types.

use wasm_bindgen::prelude::*;

/// Hash of the rules source this build was compiled from; compare with the contract's `core_version`.
#[wasm_bindgen]
pub fn rules_hash() -> String {
    ls_core::RULES_HASH.to_string()
}

/// Hash of rules.rs plus the engine (lib.rs); the contract reports the same value.
#[wasm_bindgen]
pub fn core_hash() -> String {
    ls_core::CORE_HASH.to_string()
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

/// Longest move log a dive accepts (the contract rejects longer logs).
#[wasm_bindgen]
pub fn max_moves() -> u32 {
    ls_core::MAX_MOVES
}

/// `[[id, "name"], ...]` for every item kind the rules produce; the name is also the NEP-245 token id.
#[wasm_bindgen]
pub fn item_names() -> String {
    let pairs: Vec<(u32, &str)> = ls_core::ITEM_IDS
        .iter()
        .map(|&id| (id, ls_core::item_name(id).unwrap_or("unknown")))
        .collect();
    serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".to_string())
}

/// A dive in progress. The page calls `step` once per key press and keeps the move log itself.
#[wasm_bindgen]
pub struct Sim {
    inner: ls_core::DiveSim,
}

#[wasm_bindgen]
impl Sim {
    /// `entry_json` is a `DiveEntry`: start depth, the locked loadout and the graves the dive may meet.
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u64, entry_json: &str) -> Result<Sim, JsError> {
        let entry: ls_core::DiveEntry = serde_json::from_str(entry_json).map_err(err)?;
        Ok(Sim { inner: ls_core::DiveSim::new(seed, &entry) })
    }

    /// Applies one move (0 up, 1 down, 2 left, 3 right, anything else waits); returns `StepEvents` as JSON.
    pub fn step(&mut self, mv: u8) -> Result<String, JsError> {
        serde_json::to_string(&self.inner.step(mv)).map_err(err)
    }

    /// Everything needed to draw the current state, as `DiveView` JSON.
    pub fn view(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.inner.view()).map_err(err)
    }

    pub fn is_over(&self) -> bool {
        self.inner.is_over()
    }

    /// The `DiveOutcome` so far (final once `is_over`, or whenever the player returns alive), as JSON.
    pub fn outcome(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.inner.outcome()).map_err(err)
    }
}
