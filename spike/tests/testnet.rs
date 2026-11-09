//! Same settle_dive gas measurement on the live NEAR testnet, to check the sandbox numbers.
//!
//! Core v1 flow: start_dive (the contract picks the day seed) -> dive_entry -> settle_dive(dive_id, moves).
//! Ported from the spike's settle_dive(seed, moves) on 2026-11-07 and not yet run against testnet.
//!
//! Ignored by default (it spends a little testnet NEAR and needs a funded testnet account):
//!   LASTSERVER_ACCOUNT_ID=<parent>.testnet LASTSERVER_SECRET_KEY=ed25519:... \
//!     cargo test -p spike --test testnet -- --ignored --nocapture
//! Creates a fresh contract sub-account and one sub-account per dive, runs the dives that had no graves
//! in the sandbox benchmark, writes docs/spike/GAS-TESTNET-V1.{json,md}, then deletes every account it created
//! (balances go back to the parent). Testnet only: refuses any parent that does not end with ".testnet".

use base64::Engine;
use ls_core::bot::{generate_moves, Style};
use ls_core::{run_dive, DiveEntry, DiveOutcome};
use near_workspaces::types::{Gas, NearToken, SecretKey};
use near_workspaces::{Account, AccountId};
use serde_json::{json, Value};
use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

// The sandbox benchmark attached 1000 Tgas. On testnet the signer must hold prepaid gas x gas price up front
// (1000 Tgas cost 1.0009 NEAR on 2026-11-07), so attach 100 Tgas: 7x the worst burn seen (14 Tgas).
// Burnt gas does not depend on the prepaid amount.
const PREPAID_TGAS: u64 = 100;
const EXPLORER: &str = "https://testnet.nearblocks.io/txns/";

fn wasm_path() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    target.join("wasm32-unknown-unknown/release/ls_contract.wasm")
}

#[tokio::test]
#[ignore]
async fn testnet_settle_dive_gas() -> TestResult {
    let parent: AccountId = std::env::var("LASTSERVER_ACCOUNT_ID")?.parse()?;
    assert!(parent.as_str().ends_with(".testnet"), "testnet accounts only");
    let sk: SecretKey = std::env::var("LASTSERVER_SECRET_KEY")?.parse()?;
    let run_id = std::env::var("RUN_ID").unwrap_or_else(|_| "r1".into());
    let wasm = std::fs::read(wasm_path())?;

    // rpc.testnet.near.org answered 429 TooManyRequests mid-run on 2026-11-07; RPC_URL picks another provider.
    let rpc = std::env::var("RPC_URL").unwrap_or_else(|_| "https://rpc.testnet.near.org".into());
    let worker = near_workspaces::testnet().rpc_addr(&rpc).await?;
    let root = Account::from_secret_key(parent.clone(), sk.clone(), &worker);
    let status = worker.status().await?;
    assert_eq!(status.chain_id, "testnet", "RPC {rpc} is not testnet");
    println!("rpc {rpc}: chain {} node {}", status.chain_id, status.version.version);

    // Every sub-account gets the parent's key, so anything left after a failure can still be deleted.
    let mut created: Vec<Account> = vec![];
    let result = measure(&root, &sk, &run_id, &wasm, &mut created).await;
    let mut deleted = vec![];
    for a in created.iter().rev() {
        match a.clone().delete_account(&parent).await {
            Ok(r) if r.is_success() => deleted.push(a.id().clone()),
            other => eprintln!("could not delete {}: {other:?}", a.id()),
        }
    }
    println!("deleted {} of {} accounts, balances returned to {parent}", deleted.len(), created.len());
    let (cid, runs) = result?;

    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/spike");
    let report: Value = json!({
        "generated_by": "spike/tests/testnet.rs testnet_settle_dive_gas",
        "network": "testnet", "node_version": status.version.version, "protocol_version": status.protocol_version,
        "contract_account": cid, "wasm_size_bytes": wasm.len(), "prepaid_tgas": PREPAID_TGAS,
        "runs": runs, "deleted_accounts": deleted,
    });
    std::fs::write(docs.join("GAS-TESTNET-V1.json"), serde_json::to_string_pretty(&report)?)?;
    let mut md = String::from("# settle_dive gas on live NEAR testnet\n\n");
    md.push_str(&format!("Node {} (protocol {}), wasm {} bytes, prepaid {} Tgas. Contract `{}` (deleted after the run). Same bots and seeds as the sandbox rows with no graves.\n\n",
        status.version.version, status.protocol_version, wasm.len(), PREPAID_TGAS, cid));
    md.push_str("| log | moves | seed | depth | gas, Tgas | contract = native | transaction |\n|---|---|---|---|---|---|---|\n");
    for r in report["runs"].as_array().unwrap() {
        md.push_str(&format!("| {} | {} | {} | {} | {:.3} | {} | [{}]({}) |\n", r["style"].as_str().unwrap(), r["moves_in_log"], r["seed"],
            r["final_depth"], r["tgas"].as_f64().unwrap(), if r["deterministic_equal"].as_bool().unwrap() { "yes" } else { "NO" },
            &r["tx_hash"].as_str().unwrap()[..8], r["explorer"].as_str().unwrap()));
    }
    std::fs::write(docs.join("GAS-TESTNET-V1.md"), md)?;
    Ok(())
}

/// Deletes sub-accounts left by a failed run (they carry the parent's key). CLEANUP_IDS = comma-separated ids.
#[tokio::test]
#[ignore]
async fn testnet_cleanup() -> TestResult {
    let parent: AccountId = std::env::var("LASTSERVER_ACCOUNT_ID")?.parse()?;
    assert!(parent.as_str().ends_with(".testnet"), "testnet accounts only");
    let sk: SecretKey = std::env::var("LASTSERVER_SECRET_KEY")?.parse()?;
    let rpc = std::env::var("RPC_URL").unwrap_or_else(|_| "https://rpc.testnet.near.org".into());
    let worker = near_workspaces::testnet().rpc_addr(&rpc).await?;
    let mut failed = 0;
    for id in std::env::var("CLEANUP_IDS")?.split(',').filter(|s| !s.is_empty()) {
        let id: AccountId = id.parse()?;
        assert!(id.as_str().ends_with(&format!(".{parent}")), "only sub-accounts of {parent}");
        match Account::from_secret_key(id.clone(), sk.clone(), &worker).delete_account(&parent).await {
            Ok(r) if r.is_success() => println!("deleted {id}"),
            other => { failed += 1; eprintln!("could not delete {id}: {other:?}") }
        }
    }
    assert_eq!(failed, 0, "{failed} accounts not deleted");
    Ok(())
}

async fn measure(
    root: &Account,
    sk: &SecretKey,
    run_id: &str,
    wasm: &[u8],
    created: &mut Vec<Account>,
) -> Result<(AccountId, Vec<Value>), Box<dyn std::error::Error>> {
    let c = root
        .create_subaccount(&format!("gas-{run_id}"))
        .keys(sk.clone())
        .initial_balance(NearToken::from_near(4)) // ~307 KB of code (core v1) stakes ~3.1 NEAR of storage
        .transact()
        .await?
        .into_result()?;
    created.push(c.clone());
    let contract = c.deploy(wasm).await?.into_result()?;
    contract.call("new").transact().await?.into_result()?;
    println!("contract {} deployed and initialised ({} bytes)", contract.id(), wasm.len());

    // Rows of the sandbox benchmark with no graves in state; survivors first (they never die, so no graves appear).
    // Every dive uses the contract's day seed, so the run id is the only per-row variation.
    let plan: [(Style, &str, usize, u64); 4] = [
        (Style::Survivor, "survivor", 300, 1),
        (Style::Survivor, "survivor", 1000, 1),
        (Style::Survivor, "survivor", 3000, 1),
        (Style::Rusher, "rusher", 300, 1),
    ];
    let mut runs = vec![];
    for (style, label, n, row) in plan {
        let who = root
            .create_subaccount(&format!("{label}{n}s{row}-{run_id}"))
            .keys(sk.clone())
            .initial_balance(NearToken::from_millinear(200)) // covers 100 Tgas prepaid (~0.1 NEAR)
            .transact()
            .await?
            .into_result()?;
        created.push(who.clone());
        let ticket: Value = who
            .call(contract.id(), "start_dive")
            .args_json(json!({ "loadout": [] }))
            .gas(Gas::from_tgas(PREPAID_TGAS))
            .transact()
            .await?
            .into_result()?
            .json()?;
        let seed: u64 = ticket["seed"].as_str().ok_or("ticket seed")?.parse()?;
        let entry: DiveEntry = contract.view("dive_entry").args_json(json!({ "account_id": who.id() })).await?.json()?;
        let moves = generate_moves(seed, &entry, n, style);
        let native = run_dive(seed, &entry, &moves);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&moves);
        let res = who
            .call(contract.id(), "settle_dive")
            .args_json(json!({ "dive_id": ticket["dive_id"], "moves": b64 }))
            .gas(Gas::from_tgas(PREPAID_TGAS))
            .transact()
            .await?;
        let total = res.total_gas_burnt.as_gas();
        let tx_hash = res.outcome().transaction_hash.to_string();
        let outcome: DiveOutcome = res.clone().into_result().map_err(|e| format!("settle_dive failed: {e:?}"))?.json()?;
        let equal = outcome == native;
        assert!(equal, "contract outcome differs from native run_dive");
        println!("{label} n={n} seed={seed}: depth={} graves_in={} total={:.3} Tgas tx={tx_hash}",
            outcome.final_depth, entry.graves.len(), total as f64 / 1e12);
        runs.push(json!({
            "style": label, "moves_in_log": n, "seed": seed.to_string(), "account": who.id(),
            "graves_in": entry.graves.len(), "final_depth": outcome.final_depth, "died": outcome.died,
            "total_gas_burnt": total, "tgas": total as f64 / 1e12, "deterministic_equal": equal,
            "tx_hash": tx_hash, "explorer": format!("{EXPLORER}{}", res.outcome().transaction_hash),
        }));
        // Give the deposit back right away so the parent never needs more than one player's balance.
        if who.clone().delete_account(root.id()).await?.is_success() {
            created.retain(|a| a.id() != who.id());
        }
    }
    Ok((contract.id().clone(), runs))
}
