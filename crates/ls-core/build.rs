//! Emits LS_CORE_RULES_HASH (sha256 of src/rules.rs, all the numbers) and LS_CORE_CORE_HASH
//! (sha256 of src/lib.rs followed by src/rules.rs, numbers and engine), so the contract and the
//! browser can report exactly which rules they run.

use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    println!("cargo:rerun-if-changed=src/rules.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    let rules = std::fs::read("src/rules.rs").expect("read src/rules.rs");
    let engine = std::fs::read("src/lib.rs").expect("read src/lib.rs");
    println!("cargo:rustc-env=LS_CORE_RULES_HASH={}", hex(&Sha256::digest(&rules)));
    let mut core = Sha256::new();
    core.update(&engine);
    core.update(&rules);
    println!("cargo:rustc-env=LS_CORE_CORE_HASH={}", hex(&core.finalize()));
}
