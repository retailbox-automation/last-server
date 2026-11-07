//! Emits LS_CORE_RULES_HASH: FNV-1a 64 of src/lib.rs (the rules), so the contract can report
//! exactly which rules it settles dives with.

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    let src = std::fs::read("src/lib.rs").expect("read src/lib.rs");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in src {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    println!("cargo:rustc-env=LS_CORE_RULES_HASH={h:016x}");
}
