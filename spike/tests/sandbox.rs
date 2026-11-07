//! Local-sandbox tests for the Last Server world contract (core v1) and the v1 gas benchmark.
//!
//! Build the wasm first:
//!   cargo build --target wasm32-unknown-unknown --release -p ls-contract
//! then: cargo test -p spike --release --test sandbox -- --test-threads=1 --nocapture
//!
//! Every settle_dive is checked against the native rules: the contract's outcome must equal
//! `run_dive(ticket.seed, dive_entry, moves)`. `bench_v1_gas` writes docs/spike/GAS-RESULTS-V1.json
//! and docs/spike/GAS-RESULTS-V1.md. (The spike benchmark of 87cc079 targeted the old
//! `settle_dive(seed, moves)` and is replaced by this one; its results stay in GAS-RESULTS.md.)

use base64::Engine;
use ls_core::bot::{generate_moves, Style, BOT_MAX_DEPTH};
use ls_core::rules::GRAVES_PER_DEPTH;
use ls_core::{run_dive, DiveEntry, DiveOutcome, MAX_MOVES};
use near_workspaces::network::Sandbox;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, AccountId, Contract, Worker};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

type R<T> = Result<T, Box<dyn std::error::Error>>;

/// Prepaid gas per call (burnt gas does not depend on it). Players on testnet attach 100 Tgas.
const PREPAID_TGAS: u64 = 300;
const TOKENS: [&str; 6] = ["gold", "blade", "plate", "gem", "potion", "blue_key"];

fn wasm_path() -> PathBuf {
    if let Ok(p) = std::env::var("LS_CONTRACT_WASM") {
        return PathBuf::from(p);
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    target.join("wasm32-unknown-unknown/release/ls_contract.wasm")
}

fn docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/spike")
}

fn tgas(g: u64) -> f64 {
    g as f64 / 1e12
}

struct Env {
    worker: Worker<Sandbox>,
    contract: Contract,
    wasm_len: usize,
    /// Every event of every successful call made through the helpers, in order.
    events: Mutex<Vec<Value>>,
}

#[derive(Deserialize, Clone, Debug)]
struct Ticket {
    dive_id: String,
    day: u32,
    seed: String,
    start_depth: u32,
}

impl Ticket {
    fn seed(&self) -> u64 {
        self.seed.parse().expect("seed is a u64 string")
    }
}

struct Run {
    ticket: Ticket,
    entry: DiveEntry,
    outcome: DiveOutcome,
    gas: u64,
    events: Vec<Value>,
}

async fn setup() -> R<Env> {
    let path = wasm_path();
    let wasm = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let worker = near_workspaces::sandbox().await?;
    let contract = worker.dev_deploy(&wasm).await?;
    contract.call("new").transact().await?.into_result()?;
    Ok(Env { worker, contract, wasm_len: wasm.len(), events: Mutex::new(vec![]) })
}

async fn new_account(env: &Env, name: &str) -> R<Account> {
    let root = env.worker.root_account()?;
    Ok(root.create_subaccount(name).initial_balance(NearToken::from_near(20)).transact().await?.into_result()?)
}

/// Every `EVENT_JSON:` log line, parsed.
fn events_of(res: &ExecutionFinalResult) -> Vec<Value> {
    res.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str(j).expect("event is JSON"))
        .collect()
}

/// Calls `method` and returns the raw result; events of a successful call are recorded.
async fn call(env: &Env, who: &Account, method: &str, args: Value, deposit: u128) -> R<ExecutionFinalResult> {
    let res = who
        .call(env.contract.id(), method)
        .args_json(args)
        .deposit(NearToken::from_yoctonear(deposit))
        .gas(Gas::from_tgas(PREPAID_TGAS))
        .transact()
        .await?;
    if res.is_success() {
        env.events.lock().unwrap().extend(events_of(&res));
    }
    Ok(res)
}

fn err_text(res: ExecutionFinalResult) -> String {
    match res.into_result() {
        Ok(_) => "call succeeded".into(),
        Err(e) => format!("{e:?}"),
    }
}

async fn view<T: serde::de::DeserializeOwned>(env: &Env, method: &str, args: Value) -> R<T> {
    Ok(env.contract.view(method).args_json(args).await?.json()?)
}

async fn balances(env: &Env, who: &AccountId) -> R<Vec<u128>> {
    let v: Vec<String> = view(env, "mt_batch_balance_of", json!({ "account_id": who, "token_ids": TOKENS })).await?;
    Ok(v.iter().map(|s| s.parse().unwrap()).collect())
}

fn idx(token: &str) -> usize {
    TOKENS.iter().position(|t| *t == token).expect("known token")
}

fn amount_of(list: &[(u32, u32)], token: &str) -> u128 {
    list.iter().filter(|e| ls_core::item_name(e.0) == Some(token)).map(|e| u128::from(e.1)).sum()
}

async fn start(env: &Env, who: &Account, loadout: &[(&str, u128)]) -> R<Ticket> {
    let lo: Vec<Value> = loadout.iter().map(|(t, n)| json!([t, n.to_string()])).collect();
    let res = call(env, who, "start_dive", json!({ "loadout": lo }), 0).await?;
    Ok(res.into_result().map_err(|e| format!("start_dive failed: {e:?}"))?.json()?)
}

fn settle_args(ticket: &Ticket, moves: &[u8]) -> Value {
    json!({ "dive_id": ticket.dive_id, "moves": base64::engine::general_purpose::STANDARD.encode(moves) })
}

/// settle_dive from `who`; the contract's outcome must equal the native rules on the same input.
async fn settle(env: &Env, who: &Account, ticket: &Ticket, entry: &DiveEntry, moves: &[u8]) -> R<(DiveOutcome, u64, Vec<Value>)> {
    let native = run_dive(ticket.seed(), entry, moves);
    let res = call(env, who, "settle_dive", settle_args(ticket, moves), 0).await?;
    let gas = res.total_gas_burnt.as_gas();
    let ev = events_of(&res);
    let outcome: DiveOutcome = res.into_result().map_err(|e| format!("settle_dive failed: {e:?}"))?.json()?;
    assert_eq!(outcome, native, "contract outcome differs from native run_dive");
    Ok((outcome, gas, ev))
}

/// start_dive → read dive_entry → bot log on the ticket's seed → settle_dive.
async fn dive(env: &Env, who: &Account, loadout: &[(&str, u128)], style: Style, n: usize) -> R<Run> {
    let ticket = start(env, who, loadout).await?;
    let entry: DiveEntry = view(env, "dive_entry", json!({ "account_id": who.id() })).await?;
    assert_eq!(entry.start_depth, ticket.start_depth);
    let moves = generate_moves(ticket.seed(), &entry, n, style);
    let (outcome, gas, events) = settle(env, who, &ticket, &entry, &moves).await?;
    Ok(Run { ticket, entry, outcome, gas, events })
}

fn assert_event_shapes(ev: &[Value]) {
    for e in ev {
        assert_eq!(e["version"], "1.0.0", "{e}");
        let data = e["data"].as_array().expect("data is an array");
        assert!(!data.is_empty(), "{e}");
        match e["standard"].as_str() {
            Some("nep245") => {
                for d in data {
                    let ids = d["token_ids"].as_array().expect("token_ids");
                    let amounts = d["amounts"].as_array().expect("amounts");
                    assert_eq!(ids.len(), amounts.len(), "{e}");
                    for a in amounts {
                        a.as_str().expect("amount is a string").parse::<u128>().expect("base-10");
                    }
                    match e["event"].as_str().unwrap() {
                        "mt_mint" | "mt_burn" => assert!(d["owner_id"].is_string(), "{e}"),
                        "mt_transfer" => assert!(d["old_owner_id"].is_string() && d["new_owner_id"].is_string(), "{e}"),
                        other => panic!("unexpected nep245 event {other}"),
                    }
                }
            }
            Some("lastserver") => assert!(
                ["dive_started", "dive_settled", "grave_created", "grave_looted", "grave_crumbled", "dive_expired"]
                    .contains(&e["event"].as_str().unwrap()),
                "{e}"
            ),
            _ => panic!("unexpected standard in {e}"),
        }
    }
}

fn names(ev: &[Value]) -> Vec<String> {
    ev.iter().map(|e| format!("{}:{}", e["standard"].as_str().unwrap(), e["event"].as_str().unwrap())).collect()
}

/// The contract's counters (economy, stats_by_day, supply, graves_summary, its own balances) must
/// equal the sums over every event the helpers recorded.
async fn check_counters(env: &Env) -> R<()> {
    let ev = env.events.lock().unwrap().clone();
    assert_event_shapes(&ev);
    let me = env.contract.id().as_str();
    let mut minted = BTreeMap::<String, u128>::new();
    let mut burned = BTreeMap::<String, u128>::new();
    let mut to_graves = BTreeMap::<String, u128>::new();
    let mut from_graves = BTreeMap::<String, u128>::new();
    let mut held = BTreeMap::<String, i128>::new();
    let mut days = BTreeMap::<u64, [u64; 4]>::new(); // dives, deaths, graves_looted, moves
    let mut graves_live: i64 = 0;
    for e in &ev {
        let d = &e["data"][0];
        match (e["standard"].as_str().unwrap(), e["event"].as_str().unwrap()) {
            ("nep245", kind) => {
                for row in e["data"].as_array().unwrap() {
                    let ids = row["token_ids"].as_array().unwrap();
                    let amounts = row["amounts"].as_array().unwrap();
                    for (t, a) in ids.iter().zip(amounts) {
                        let t = t.as_str().unwrap().to_string();
                        let a: u128 = a.as_str().unwrap().parse()?;
                        match kind {
                            "mt_mint" => *minted.entry(t).or_default() += a,
                            "mt_burn" => {
                                if row["owner_id"] == me {
                                    *held.entry(t.clone()).or_default() -= a as i128;
                                }
                                *burned.entry(t).or_default() += a;
                            }
                            "mt_transfer" => {
                                if row["new_owner_id"] == me {
                                    *to_graves.entry(t.clone()).or_default() += a;
                                    *held.entry(t).or_default() += a as i128;
                                } else if row["old_owner_id"] == me {
                                    *from_graves.entry(t.clone()).or_default() += a;
                                    *held.entry(t).or_default() -= a as i128;
                                }
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
            ("lastserver", "dive_started") => days.entry(d["day"].as_u64().unwrap()).or_default()[0] += 1,
            ("lastserver", "dive_settled") => {
                let row = days.entry(d["day"].as_u64().unwrap()).or_default();
                row[1] += u64::from(d["died"].as_bool().unwrap());
                row[3] += d["moves_applied"].as_u64().unwrap();
            }
            ("lastserver", "dive_expired") => days.entry(d["day"].as_u64().unwrap()).or_default()[1] += 1,
            ("lastserver", "grave_looted") => {
                let n = d["graves"].as_array().unwrap().len();
                graves_live -= n as i64;
                // graves_looted is counted on the looting dive's ticket day, reported in dive_settled.
                let _ = n;
            }
            ("lastserver", "grave_created") => graves_live += 1,
            ("lastserver", "grave_crumbled") => graves_live -= 1,
            _ => {}
        }
    }
    // graves_looted per day: from dive_settled.looted_graves of that day.
    for e in ev.iter().filter(|e| e["event"] == "dive_settled") {
        let d = &e["data"][0];
        days.entry(d["day"].as_u64().unwrap()).or_default()[2] += d["looted_graves"].as_array().unwrap().len() as u64;
    }
    let me_id: AccountId = me.parse()?;
    let held_now = balances(env, &me_id).await?;
    for t in TOKENS {
        let e: Value = view(env, "economy", json!({ "token_id": t })).await?;
        let g = |m: &BTreeMap<String, u128>| m.get(t).copied().unwrap_or(0).to_string();
        assert_eq!(e["minted"], g(&minted), "{t} minted");
        assert_eq!(e["burned"], g(&burned), "{t} burned");
        assert_eq!(e["to_graves"], g(&to_graves), "{t} to_graves");
        assert_eq!(e["from_graves"], g(&from_graves), "{t} from_graves");
        let supply: Option<String> = view(env, "mt_supply", json!({ "token_id": t })).await?;
        let expect = minted.get(t).copied().unwrap_or(0) - burned.get(t).copied().unwrap_or(0);
        assert_eq!(supply.unwrap().parse::<u128>()?, expect, "{t} supply");
        assert_eq!(held_now[idx(t)] as i128, held.get(t).copied().unwrap_or(0), "{t} held by the contract");
    }
    for (day, [dives, deaths, looted, moves]) in &days {
        let s: Value = view(env, "stats_by_day", json!({ "day": day })).await?;
        assert_eq!(s["dives"], *dives, "day {day} dives");
        assert_eq!(s["deaths"], *deaths, "day {day} deaths");
        assert_eq!(s["graves_looted"], *looted, "day {day} graves_looted");
        assert_eq!(s["moves"], moves.to_string(), "day {day} moves");
    }
    let summary: Vec<(u32, u32)> = view(env, "graves_summary", json!({})).await?;
    assert_eq!(summary.iter().map(|r| i64::from(r.1)).sum::<i64>(), graves_live, "graves_summary vs events");
    println!("counters match {} events over {} day(s)", ev.len(), days.len());
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn survive_banks_finds_and_locked_items_stay_put() -> R<()> {
    let env = setup().await?;
    let alice = new_account(&env, "alice").await?;
    let bob = new_account(&env, "bob").await?;

    let day: u32 = view(&env, "current_day", json!({})).await?;
    let seed: String = view(&env, "day_seed", json!({ "day": day })).await?;

    // 1. A fresh player survives a dive: finds are minted into the stash, used-up ones burned.
    let run = dive(&env, &alice, &[], Style::Survivor, 600).await?;
    assert!(!run.outcome.died);
    assert_eq!(run.ticket.day, day);
    assert_eq!(run.ticket.seed, seed, "the ticket carries the contract's day seed");
    assert!(!run.outcome.picked.is_empty());
    let names = names(&run.events);
    println!("survive events: {names:?}");
    assert_eq!(names.first().map(String::as_str), Some("nep245:mt_mint"));
    assert_eq!(names.last().map(String::as_str), Some("lastserver:dive_settled"));
    let have = balances(&env, alice.id()).await?;
    for t in TOKENS {
        let expect = amount_of(&run.outcome.picked, t) - amount_of(&run.outcome.burned, t);
        assert_eq!(have[idx(t)], expect, "{t}");
    }
    let active: Option<Value> = view(&env, "active_dive", json!({ "account_id": alice.id() })).await?;
    assert!(active.is_none());
    let p: Value = view(&env, "player", json!({ "account_id": alice.id() })).await?;
    assert_eq!(p["dives"], 1);
    assert_eq!(p["deaths"], 0);
    assert_eq!(p["deepest"], run.outcome.final_depth);
    assert!(p["locked"].as_array().unwrap().is_empty());

    // 2. Carry some gold: it stays alice's balance but is locked during the dive.
    let gold = have[idx("gold")];
    assert!(gold >= 4, "alice found gold: {gold}");
    let carry = gold / 2;
    let ticket = start(&env, &alice, &[("gold", carry)]).await?;
    let active: Option<Ticket> = view(&env, "active_dive", json!({ "account_id": alice.id() })).await?;
    assert_eq!(active.unwrap().dive_id, ticket.dive_id);
    let p: Value = view(&env, "player", json!({ "account_id": alice.id() })).await?;
    assert_eq!(p["locked"], json!([["gold", carry.to_string()]]));
    assert_eq!(balances(&env, alice.id()).await?[idx("gold")], gold, "locked still counts in mt_balance_of");
    let free = gold - carry;
    let res = call(&env, &alice, "mt_transfer", json!({ "receiver_id": bob.id(), "token_id": "gold", "amount": (free + 1).to_string() }), 1).await?;
    let err = err_text(res);
    assert!(err.contains("items carried in an active dive cannot be transferred"), "{err}");
    let res = call(&env, &alice, "mt_transfer", json!({ "receiver_id": bob.id(), "token_id": "gold", "amount": (gold + 1).to_string() }), 1).await?;
    assert!(err_text(res).contains("insufficient balance"));
    let res = call(&env, &alice, "mt_transfer", json!({ "receiver_id": bob.id(), "token_id": "gold", "amount": free.to_string() }), 1).await?;
    res.into_result()?;
    assert_eq!(balances(&env, alice.id()).await?[idx("gold")], carry);
    assert_eq!(balances(&env, bob.id()).await?[idx("gold")], free);

    // 3. Settle the dive alive: the carried gold is free again.
    let entry: DiveEntry = view(&env, "dive_entry", json!({ "account_id": alice.id() })).await?;
    assert_eq!(entry.inventory, vec![(ls_core::ITEM_GOLD, carry as u32)]);
    let moves = generate_moves(ticket.seed(), &entry, 300, Style::Survivor);
    let (out, _, _) = settle(&env, &alice, &ticket, &entry, &moves).await?;
    assert!(!out.died);
    let p: Value = view(&env, "player", json!({ "account_id": alice.id() })).await?;
    assert!(p["locked"].as_array().unwrap().is_empty());
    let g = balances(&env, alice.id()).await?[idx("gold")];
    assert_eq!(g, carry + amount_of(&out.picked, "gold") - amount_of(&out.burned, "gold"));
    let res = call(&env, &alice, "mt_transfer", json!({ "receiver_id": bob.id(), "token_id": "gold", "amount": g.to_string() }), 1).await?;
    res.into_result()?;

    // NEP-245 odds and ends: 1 yocto, the contract refuses transfers, views.
    let res = bob.call(env.contract.id(), "mt_transfer").args_json(json!({ "receiver_id": alice.id(), "token_id": "gold", "amount": "1" })).transact().await?;
    assert!(err_text(res).contains("Requires attached deposit of exactly 1 yoctoNEAR"));
    let res = call(&env, &bob, "mt_transfer", json!({ "receiver_id": env.contract.id(), "token_id": "gold", "amount": "1" }), 1).await?;
    assert!(err_text(res).contains("the world contract does not take transfers"));
    let res = call(&env, &bob, "mt_batch_transfer", json!({ "receiver_id": alice.id(), "token_ids": ["gold"], "amounts": ["1", "1"] }), 1).await?;
    assert!(err_text(res).contains("token_ids and amounts differ in length"));
    let res = call(&env, &bob, "mt_batch_transfer", json!({ "receiver_id": alice.id(), "token_ids": ["gold", "gold"], "amounts": ["1", "1"], "approvals": [null, null] }), 1).await?;
    let ok = res.into_result()?;
    let ev = events_of_success(&ok);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0]["event"], "mt_transfer");
    assert_eq!(ev[0]["data"][0]["amounts"], json!(["1", "1"]));
    let tokens: Value = view(&env, "mt_token", json!({ "token_ids": ["gold", "blue_key", "sword"] })).await?;
    assert_eq!(tokens, json!([{ "token_id": "gold", "owner_id": null }, { "token_id": "blue_key", "owner_id": null }, null]));
    let sup: Vec<Option<String>> = view(&env, "mt_batch_supply", json!({ "token_ids": ["gold", "sword"] })).await?;
    assert!(sup[0].is_some() && sup[1].is_none());

    let v: Value = view(&env, "core_version", json!({})).await?;
    assert_eq!(v["rules_sha256"], ls_core::RULES_HASH);
    assert_eq!(v["core_sha256"], ls_core::CORE_HASH);
    assert_eq!(v["ls_core"], ls_core::VERSION);
    println!("core_version: {v}");
    check_counters(&env).await
}

fn events_of_success(ok: &near_workspaces::result::ExecutionSuccess) -> Vec<Value> {
    ok.logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str(j).expect("event is JSON"))
        .collect()
}

#[tokio::test]
async fn death_makes_a_grave_and_another_player_loots_it() -> R<()> {
    let env = setup().await?;
    let alice = new_account(&env, "alice").await?;
    let bob = new_account(&env, "bob").await?;
    let me = env.contract.id().clone();

    let first = dive(&env, &alice, &[], Style::Survivor, 600).await?;
    assert!(!first.outcome.died);
    let stash = balances(&env, alice.id()).await?;
    let loadout: Vec<(&str, u128)> = TOKENS.iter().map(|t| (*t, stash[idx(t)])).filter(|e| e.1 > 0).collect();
    println!("alice carries {loadout:?}");

    // Alice walks into the monsters carrying everything: she dies, her items become a grave.
    let second = dive(&env, &alice, &loadout, Style::Doomed, 600).await?;
    assert!(second.outcome.died);
    let names = names(&second.events);
    println!("death events: {names:?}");
    assert!(names.contains(&"lastserver:grave_created".to_string()));
    assert_eq!(names.last().map(String::as_str), Some("lastserver:dive_settled"));
    let settled = second.events.last().unwrap();
    let grave_id = settled["data"][0]["grave_id"].as_u64().expect("grave id") as u32;
    assert_eq!(balances(&env, alice.id()).await?, vec![0; TOKENS.len()]);
    let depth = second.outcome.final_depth;
    let graves: Vec<Value> = view(&env, "graves", json!({ "depth": depth })).await?;
    assert_eq!(graves.len(), 1);
    assert_eq!(graves[0]["id"], grave_id);
    assert_eq!(graves[0]["owner_id"], alice.id().as_str());
    let (gx, gy) = second.outcome.death_pos.unwrap();
    assert_eq!((graves[0]["x"].as_u64().unwrap() as u8, graves[0]["y"].as_u64().unwrap() as u8), (gx, gy));
    let held = balances(&env, &me).await?;
    for t in TOKENS {
        assert_eq!(held[idx(t)], amount_of(&second.outcome.dropped_into_grave, t), "contract holds {t}");
    }
    let summary: Vec<(u32, u32)> = view(&env, "graves_summary", json!({})).await?;
    assert_eq!(summary, vec![(depth, 1)]);
    let p: Value = view(&env, "player", json!({ "account_id": alice.id() })).await?;
    assert_eq!(p["deaths"], 1);

    // Bob dives the same day (same map), reaches the grave and loots it.
    let bob_entry: DiveEntry = view(&env, "dive_entry", json!({ "account_id": bob.id() })).await?;
    assert_eq!(bob_entry.graves.len(), 1);
    assert_eq!(bob_entry.graves[0].id, grave_id);
    let third = dive(&env, &bob, &[], Style::Looter, 600).await?;
    assert_eq!(third.outcome.looted_graves, vec![grave_id]);
    assert!(!third.outcome.died);
    let looted = third.events.iter().find(|e| e["event"] == "grave_looted").expect("grave_looted");
    assert_eq!(looted["data"][0]["looter_id"], bob.id().as_str());
    assert_eq!(looted["data"][0]["graves"][0]["owner_id"], alice.id().as_str());
    let loot = third
        .events
        .iter()
        .find(|e| e["event"] == "mt_transfer" && e["data"][0]["old_owner_id"] == me.as_str())
        .expect("loot transfer");
    assert_eq!(loot["data"][0]["new_owner_id"], bob.id().as_str());
    assert_eq!(balances(&env, &me).await?, vec![0; TOKENS.len()], "grave emptied");
    let bob_after = balances(&env, bob.id()).await?;
    for t in TOKENS {
        let expect = amount_of(&second.outcome.dropped_into_grave, t) + amount_of(&third.outcome.picked, t)
            - amount_of(&third.outcome.burned, t);
        assert_eq!(bob_after[idx(t)], expect, "bob {t}");
    }
    let graves: Vec<Value> = view(&env, "graves", json!({ "depth": depth })).await?;
    assert!(graves.is_empty());
    let summary: Vec<(u32, u32)> = view(&env, "graves_summary", json!({})).await?;
    assert!(summary.is_empty());
    check_counters(&env).await
}

#[tokio::test]
async fn blue_key_opens_the_vault_and_mints_a_gem() -> R<()> {
    let env = setup().await?;
    let tom = new_account(&env, "tom").await?;
    // Dive until a blue key is in the stash (keys drop from depth 5; a deep dive usually finds
    // one and may already open the depth-7 vault on the way).
    let mut gem_from_vault = false;
    for attempt in 0..3 {
        let stash = balances(&env, tom.id()).await?;
        let key = stash[idx("blue_key")];
        let loadout: Vec<(&str, u128)> = if key > 0 { vec![("blue_key", 1), ("blade", stash[idx("blade")])] } else { vec![] };
        let loadout: Vec<(&str, u128)> = loadout.into_iter().filter(|e| e.1 > 0).collect();
        let run = dive(&env, &tom, &loadout, Style::Survivor, 3000).await?;
        println!(
            "attempt {attempt}: loadout {loadout:?} depth {} died {} vault {} keys_used {} picked {:?}",
            run.outcome.final_depth, run.outcome.died, run.outcome.vault_opened, run.outcome.keys_used, run.outcome.picked
        );
        if run.outcome.vault_opened {
            assert_eq!(run.outcome.keys_used, 1);
            assert!(amount_of(&run.outcome.picked, "gem") >= 1);
            assert!(amount_of(&run.outcome.burned, "blue_key") == 1);
            let settled = run.events.last().unwrap();
            assert_eq!(settled["data"][0]["vault_opened"], true);
            let mint = run.events.iter().find(|e| e["event"] == "mt_mint").unwrap();
            assert!(mint["data"][0]["token_ids"].as_array().unwrap().contains(&json!("gem")));
            gem_from_vault = true;
            break;
        }
    }
    assert!(gem_from_vault, "no vault opened in 3 dives");
    assert!(balances(&env, tom.id()).await?[idx("gem")] >= 1);
    let e: Value = view(&env, "economy", json!({ "token_id": "blue_key" })).await?;
    assert!(e["burned"].as_str().unwrap().parse::<u128>()? >= 1);
    check_counters(&env).await
}

#[tokio::test]
async fn an_open_dive_expires_into_a_grave_the_next_day() -> R<()> {
    let env = setup().await?;
    let alice = new_account(&env, "alice").await?;
    let bob = new_account(&env, "bob").await?;
    let first = dive(&env, &alice, &[], Style::Survivor, 600).await?;
    assert!(!first.outcome.died);
    let gold = balances(&env, alice.id()).await?[idx("gold")];
    let ticket = start(&env, &alice, &[("gold", gold)]).await?;

    // Same day: nobody can expire it.
    let res = call(&env, &bob, "expire_dive", json!({ "account_id": alice.id() }), 0).await?;
    let err = err_text(res);
    assert!(err.contains("the dive's day is not over yet"), "{err}");

    // Move the sandbox clock past the day boundary.
    let day0: u32 = view(&env, "current_day", json!({})).await?;
    let t0 = std::time::Instant::now();
    let mut blocks = 0u64;
    loop {
        env.worker.fast_forward(20_000).await?;
        blocks += 20_000;
        let day: u32 = view(&env, "current_day", json!({})).await?;
        if day > day0 {
            println!("day {day0} -> {day} after fast_forward({blocks}) in {:.1}s", t0.elapsed().as_secs_f64());
            break;
        }
        assert!(blocks < 1_000_000, "fast_forward did not move the day");
    }
    let res = call(&env, &bob, "expire_dive", json!({ "account_id": alice.id() }), 0).await?;
    let ev = events_of(&res);
    res.into_result()?;
    let names = names(&ev);
    println!("expire events: {names:?}");
    let expired = ev.iter().find(|e| e["event"] == "dive_expired").expect("dive_expired");
    assert_eq!(expired["data"][0]["account_id"], alice.id().as_str());
    assert_eq!(expired["data"][0]["expired_by"], bob.id().as_str());
    assert_eq!(expired["data"][0]["dive_id"], ticket.dive_id);
    let grave_id = expired["data"][0]["grave_id"].as_u64().expect("grave") as u32;
    let graves: Vec<Value> = view(&env, "graves", json!({ "depth": ticket.start_depth })).await?;
    assert_eq!(graves.len(), 1);
    assert_eq!(graves[0]["id"], grave_id);
    assert_eq!(graves[0]["items"], json!([["gold", gold.to_string()]]), "the whole loadout, no toll");
    let (x, y) = ls_core::expired_grave_spot(ticket.seed(), ticket.dive_id.parse()?);
    assert_eq!((graves[0]["x"].as_u64().unwrap() as u8, graves[0]["y"].as_u64().unwrap() as u8), (x, y));
    assert_eq!(balances(&env, alice.id()).await?[idx("gold")], 0);
    let active: Option<Value> = view(&env, "active_dive", json!({ "account_id": alice.id() })).await?;
    assert!(active.is_none());
    let p: Value = view(&env, "player", json!({ "account_id": alice.id() })).await?;
    assert_eq!(p["deaths"], 1);
    assert!(p["locked"].as_array().unwrap().is_empty());
    // A settle after expiry has nothing to settle.
    let res = call(&env, &alice, "settle_dive", settle_args(&ticket, &[0, 1]), 0).await?;
    assert!(err_text(res).contains("no active dive for this account"));
    // Expiring twice fails too.
    let res = call(&env, &bob, "expire_dive", json!({ "account_id": alice.id() }), 0).await?;
    assert!(err_text(res).contains("no active dive for this account"));
    // The new day has a new seed.
    let next = start(&env, &alice, &[]).await?;
    assert_ne!(next.seed, ticket.seed);
    assert!(next.day > ticket.day);
    check_counters(&env).await
}

#[tokio::test]
async fn graves_per_depth_are_capped_and_the_oldest_crumbles() -> R<()> {
    let env = setup().await?;
    let me = env.contract.id().clone();
    let mule = new_account(&env, "mule").await?;
    // The mule finds some gold once, then dies again and again at depth 1 carrying 1 gold.
    let mut found = 0;
    for _ in 0..4 {
        let r = dive(&env, &mule, &[], Style::DieAt(BOT_MAX_DEPTH), 1500).await?;
        assert!(!r.outcome.died);
        found = balances(&env, mule.id()).await?[idx("gold")];
        if found > 10 {
            break;
        }
    }
    assert!(found > 10, "mule found {found} gold");
    // Bots repeat themselves on one day's map and a grave can wall a bot off at one depth, so fill
    // depth 6, then 5, then 4 (deep first: the depths a dive passes on the way down stay empty)
    // until one of them goes over the cap.
    let mut created: Vec<(u32, u32)> = vec![]; // (grave id, depth)
    let mut crumbled = None;
    let mut i = 0;
    'fill: for target in [6u32, 5, 4] {
        let mut no_growth = 0;
        while no_growth < 12 {
            i += 1;
            let gold = balances(&env, mule.id()).await?[idx("gold")];
            let loadout: Vec<(&str, u128)> = if gold > 0 { vec![("gold", 1)] } else { vec![] };
            let r = dive(&env, &mule, &loadout, Style::DieAt(target), 400 + 250 * target as usize).await?;
            let Some(id) = r.events.last().unwrap()["data"][0]["grave_id"].as_u64() else {
                no_growth += 1;
                continue;
            };
            created.push((id as u32, r.outcome.final_depth));
            if r.outcome.final_depth != target || !r.outcome.looted_graves.is_empty() {
                no_growth += 1;
            }
            if let Some(c) = r.events.iter().find(|e| e["event"] == "grave_crumbled") {
                crumbled = Some((i, c.clone(), r.events.clone(), r.outcome.final_depth));
                break 'fill;
            }
        }
    }
    let (i, c, ev, depth) = crumbled.expect("a depth went over the cap");
    let made_here: Vec<u32> = created.iter().filter(|g| g.1 == depth).map(|g| g.0).collect();
    println!("dive {i}: depth {depth}, graves made there {made_here:?}, crumbled {}", c["data"][0]);
    let burn = ev.iter().find(|e| e["event"] == "mt_burn" && e["data"][0]["owner_id"] == me.as_str()).expect("crumble burn");
    assert_eq!(burn["data"][0]["memo"], "grave crumbled");
    let graves: Vec<Value> = view(&env, "graves", json!({ "depth": depth })).await?;
    assert_eq!(graves.len() as u32, GRAVES_PER_DEPTH, "the depth is back at the cap");
    let ids: Vec<u32> = graves.iter().map(|g| g["id"].as_u64().unwrap() as u32).collect();
    let gone = c["data"][0]["grave_id"].as_u64().unwrap() as u32;
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "oldest first: {ids:?}");
    assert!(ids.iter().all(|&id| id > gone), "the oldest grave crumbled: {gone} vs {ids:?}");
    assert_eq!(*ids.last().unwrap(), *made_here.last().unwrap(), "the new grave is the newest");
    let summary: Vec<(u32, u32)> = view(&env, "graves_summary", json!({})).await?;
    assert!(summary.contains(&(depth, GRAVES_PER_DEPTH)), "{summary:?}");
    check_counters(&env).await
}

#[tokio::test]
async fn settle_guards() -> R<()> {
    let env = setup().await?;
    let alice = new_account(&env, "alice").await?;
    let bob = new_account(&env, "bob").await?;

    // Loadout checks.
    let res = call(&env, &alice, "start_dive", json!({ "loadout": [["gold", "1"]] }), 0).await?;
    assert!(err_text(res).contains("insufficient balance for loadout"));
    let res = call(&env, &alice, "start_dive", json!({ "loadout": [["sword", "1"]] }), 0).await?;
    assert!(err_text(res).contains("unknown token_id"));
    let res = call(&env, &alice, "start_dive", json!({ "loadout": [["gold", "0"]] }), 0).await?;
    assert!(err_text(res).contains("amount must be positive"));

    let ticket = start(&env, &alice, &[]).await?;
    // One active dive per account.
    let res = call(&env, &alice, "start_dive", json!({ "loadout": [] }), 0).await?;
    assert!(err_text(res).contains("an active dive already exists"));
    // Somebody else cannot settle it: bob has no dive, and with his own dive the id does not match.
    let res = call(&env, &bob, "settle_dive", settle_args(&ticket, &[1, 1]), 0).await?;
    assert!(err_text(res).contains("no active dive for this account"));
    let bob_ticket = start(&env, &bob, &[]).await?;
    let res = call(&env, &bob, "settle_dive", settle_args(&ticket, &[1, 1]), 0).await?;
    assert!(err_text(res).contains("dive_id is not this account's active dive"));
    // More than MAX_MOVES moves: rejected, the dive stays open.
    let res = call(&env, &alice, "settle_dive", settle_args(&ticket, &vec![4u8; MAX_MOVES as usize + 1]), 0).await?;
    let err = err_text(res);
    assert!(err.contains("move log longer than MAX_MOVES"), "{err}");
    let active: Option<Ticket> = view(&env, "active_dive", json!({ "account_id": alice.id() })).await?;
    assert_eq!(active.unwrap().dive_id, ticket.dive_id);
    // Exactly MAX_MOVES is fine (a bot log, so it is a real dive).
    let entry: DiveEntry = view(&env, "dive_entry", json!({ "account_id": alice.id() })).await?;
    let moves = generate_moves(ticket.seed(), &entry, MAX_MOVES as usize, Style::Survivor);
    let (out, gas, _) = settle(&env, &alice, &ticket, &entry, &moves).await?;
    println!("4000-move survivor: applied {} depth {} died {} gas {:.2} Tgas", out.moves_applied, out.final_depth, out.died, tgas(gas));
    // Settling twice: nothing left to settle.
    let res = call(&env, &alice, "settle_dive", settle_args(&ticket, &[1]), 0).await?;
    assert!(err_text(res).contains("no active dive for this account"));
    let entry: DiveEntry = view(&env, "dive_entry", json!({ "account_id": bob.id() })).await?;
    let (bob_out, _, _) = settle(&env, &bob, &bob_ticket, &entry, &[]).await?;
    assert_eq!(bob_out.moves_applied, 0);
    check_counters(&env).await
}

// ---------------------------------------------------------------------------------------------
// Gas benchmark
// ---------------------------------------------------------------------------------------------

/// Puts `total` graves on depths 10 down to 1 and then 11 onwards, `per_depth` each, by letting
/// `digger` die there (deep first, so the depths a dive passes on the way down are still empty)
/// (DieAt bots walk around graves; the digger carries 1 gold when it has some, so even a death
/// at depth 1 leaves a grave). A depth where `per_depth * 2 + 6` dives in a row add nothing (a
/// grave walls the bot off on this day's map) is skipped for the next one. Returns the dives.
async fn fill_graves(env: &Env, digger: &Account, per_depth: u32, total: u32, skipped: &mut Vec<u32>) -> R<u32> {
    let mut dives = 0;
    let mut stalled = (0u32, 0u32); // (depth, dives without progress)
    loop {
        let summary: Vec<(u32, u32)> = view(env, "graves_summary", json!({})).await?;
        let count = |d: u32| summary.iter().find(|r| r.0 == d).map_or(0, |r| r.1);
        let in_state: u32 = summary.iter().map(|r| r.1).sum();
        if in_state >= total {
            return Ok(dives);
        }
        let target = (1..=10u32)
            .rev()
            .chain(11..=60)
            .find(|&d| !skipped.contains(&d) && count(d) < per_depth)
            .ok_or("no depth left")?;
        if stalled.0 == target {
            stalled.1 += 1;
            if stalled.1 > per_depth * 2 + 6 {
                println!("depth {target} stalled at {} graves: skipped", count(target));
                skipped.push(target);
                continue;
            }
        } else {
            stalled = (target, 0);
        }
        dives += 1;
        if dives > 500 {
            return Err(format!("could not fill graves: {summary:?}").into());
        }
        let gold = balances(env, digger.id()).await?[idx("gold")];
        let loadout: Vec<(&str, u128)> = if gold > 0 { vec![("gold", 1)] } else { vec![] };
        let n = (400 + 250 * target as usize).min(MAX_MOVES as usize);
        let before = count(target);
        dive(env, digger, &loadout, Style::DieAt(target), n).await?;
        let after: Vec<(u32, u32)> = view(env, "graves_summary", json!({})).await?;
        if after.iter().find(|r| r.0 == target).map_or(0, |r| r.1) > before {
            stalled = (target, 0);
        }
    }
}

#[tokio::test]
async fn bench_v1_gas() -> R<()> {
    let env = setup().await?;
    let status = env.worker.status().await?;
    let mut rows: Vec<Value> = vec![];
    let mut record = |label: &str, n: usize, graves_in_state: u32, run: &Run| {
        let visited: u32 = run.entry.graves.iter().filter(|g| g.depth <= run.outcome.final_depth).count() as u32;
        println!(
            "{label} n={n} graves_in_state={graves_in_state} visited_depth_graves={visited} looted={} applied={} depth={} died={} gas={:.3} Tgas",
            run.outcome.looted_graves.len(),
            run.outcome.moves_applied,
            run.outcome.final_depth,
            run.outcome.died,
            tgas(run.gas)
        );
        rows.push(json!({
            "style": label,
            "n": n,
            "graves_in_state": graves_in_state,
            "graves_on_visited_depths": visited,
            "graves_looted": run.outcome.looted_graves.len(),
            "moves_applied": run.outcome.moves_applied,
            "final_depth": run.outcome.final_depth,
            "died": run.outcome.died,
            "total_gas_burnt": run.gas,
            "tgas": tgas(run.gas),
            "seed": run.ticket.seed,
            "contract_equals_native": true,
        }));
    };

    // 1. No graves on the server.
    for (style, label) in [(Style::Survivor, "survivor"), (Style::Rusher, "rusher")] {
        for n in [300usize, 1000, 3000] {
            let who = new_account(&env, &format!("{label}{n}")).await?;
            let run = dive(&env, &who, &[], style, n).await?;
            record(label, n, 0, &run);
        }
    }

    // 2. 100 graves: GRAVES_PER_DEPTH on each of depths 1..=10 (deeper ones if a depth stalls),
    //    refilled before every row.
    const TOTAL: u32 = 100;
    let digger = new_account(&env, "digger").await?;
    // Collect gold first, so even its depth-1 deaths leave graves.
    let warm = dive(&env, &digger, &[], Style::DieAt(BOT_MAX_DEPTH), 1500).await?;
    assert!(!warm.outcome.died && balances(&env, digger.id()).await?[idx("gold")] > 50);
    let mut skipped: Vec<u32> = vec![];
    let t0 = std::time::Instant::now();
    let fill_dives = fill_graves(&env, &digger, GRAVES_PER_DEPTH, TOTAL, &mut skipped).await?;
    let first_fill: Vec<(u32, u32)> = view(&env, "graves_summary", json!({})).await?;
    println!("filled {TOTAL} graves with {fill_dives} dives in {:.0}s: {first_fill:?}", t0.elapsed().as_secs_f64());
    let mut summaries: Vec<Value> = vec![];
    let cases = [
        (Style::DieAt(BOT_MAX_DEPTH), "avoider", 300usize),
        (Style::Rusher, "rusher", 300),
        (Style::Survivor, "survivor", 300),
        (Style::Looter, "looter", 300),
        (Style::Rusher, "rusher", 1000),
        (Style::Survivor, "survivor", 1000),
        (Style::Survivor, "survivor", 3000),
    ];
    for (i, (style, label, n)) in cases.into_iter().enumerate() {
        fill_graves(&env, &digger, GRAVES_PER_DEPTH, TOTAL, &mut skipped).await?;
        let summary: Vec<(u32, u32)> = view(&env, "graves_summary", json!({})).await?;
        let in_state: u32 = summary.iter().map(|r| r.1).sum();
        let who = new_account(&env, &format!("g{i}{label}{n}")).await?;
        let run = dive(&env, &who, &[], style, n).await?;
        record(label, n, in_state, &run);
        summaries.push(json!({ "row": i, "graves_summary": summary }));
    }
    drop(record);

    let worst = |n: usize, with_graves: bool| {
        rows.iter()
            .filter(|r| r["n"] == n && (r["graves_in_state"].as_u64().unwrap() > 0) == with_graves)
            .map(|r| r["tgas"].as_f64().unwrap())
            .fold(0.0, f64::max)
    };
    let worst_300_graves = worst(300, true);
    let pass = worst_300_graves <= 10.0;
    let results = json!({
        "generated_by": "spike/tests/sandbox.rs bench_v1_gas",
        "unix_time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        "sandbox": {
            "node_version": status.version.version,
            "node_build": status.version.build,
            "protocol_version": status.protocol_version,
        },
        "wasm": { "file": "<CARGO_TARGET_DIR>/wasm32-unknown-unknown/release/ls_contract.wasm", "size_bytes": env.wasm_len },
        "core": { "ls_core": ls_core::VERSION, "rules_sha256": ls_core::RULES_HASH, "core_sha256": ls_core::CORE_HASH },
        "prepaid_tgas": PREPAID_TGAS,
        "graves_per_depth": GRAVES_PER_DEPTH,
        "first_fill": { "dives": fill_dives, "graves_summary": first_fill },
        "depths_skipped_while_filling": skipped,
        "graves_summary_before_each_graves_row": summaries,
        "rows": rows,
        "threshold": { "n": 300, "graves_in_state": 100, "limit_tgas": 10, "worst_tgas": worst_300_graves, "pass": pass },
    });
    std::fs::create_dir_all(docs_dir())?;
    std::fs::write(docs_dir().join("GAS-RESULTS-V1.json"), serde_json::to_string_pretty(&results)?)?;

    let mut md = String::new();
    md.push_str("# settle_dive gas, core v1, local sandbox\n\n");
    md.push_str(&format!(
        "Generated by `spike/tests/sandbox.rs` (`bench_v1_gas`). Sandbox node {} (protocol {}), wasm {} bytes, ls-core {} (rules sha256 `{}`), prepaid {} Tgas per call. Every dive is `start_dive` + `settle_dive` on the contract's day seed from a fresh account with an empty loadout, and every outcome equals the native `run_dive` on the same input. Raw numbers: `GAS-RESULTS-V1.json`.\n\n",
        status.version.version, status.protocol_version, env.wasm_len, ls_core::VERSION, ls_core::RULES_HASH, PREPAID_TGAS
    ));
    md.push_str("| bot | moves (applied) | graves on the server | graves on the depths it entered | graves looted | depth reached | died | settle_dive gas, Tgas |\n|---|---|---|---|---|---|---|---|\n");
    for r in &rows {
        md.push_str(&format!(
            "| {} | {} ({}) | {} | {} | {} | {} | {} | {:.2} |\n",
            r["style"].as_str().unwrap(),
            r["n"],
            r["moves_applied"],
            r["graves_in_state"],
            r["graves_on_visited_depths"],
            r["graves_looted"],
            r["final_depth"],
            if r["died"].as_bool().unwrap() { "yes" } else { "no" },
            r["tgas"].as_f64().unwrap(),
        ));
    }
    md.push_str(&format!(
        "\nBots: survivor fights, drinks, rests, collects items and graves, opens the vault with a key, descends to depth {BOT_MAX_DEPTH}; looter goes for graves first; rusher runs for the stairs (most new maps per move); avoider plays like survivor but walks around graves (it reads every grave on its depths and loots none). Before each row with graves the server is refilled to {TOTAL} graves, {GRAVES_PER_DEPTH} per depth on depths 10 down to 1, then 11 onwards (a depth where the filling bot gets walled off by a grave on this day's map is skipped: {skipped:?}). Graves per depth at the first fill: {first_fill:?}; before every row: `graves_summary_before_each_graves_row` in the JSON.\n"
    ));
    md.push_str(&format!(
        "\nThreshold: 300 moves with 100 graves on the server must cost at most 10 Tgas. Worst such run: {worst_300_graves:.2} Tgas, {}. Worst 300-move run without graves: {:.2} Tgas; worst 1000: {:.2} / {:.2} Tgas (no graves / graves); worst 3000: {:.2} / {:.2} Tgas.\n",
        if pass { "PASS" } else { "FAIL" },
        worst(300, false),
        worst(1000, false),
        worst(1000, true),
        worst(3000, false),
        worst(3000, true),
    ));
    md.push_str(concat!(
        "\n## Reproduce\n\n",
        "```sh\n",
        "# toolchain: rustc with the wasm32-unknown-unknown target; .cargo/config.toml adds --cfg near for wasm\n",
        "cargo test -p ls-core --features serde\n",
        "cargo build --target wasm32-unknown-unknown --release -p ls-contract\n",
        "cargo test -p spike --release --test sandbox -- --test-threads=1 --nocapture   # downloads near-sandbox on first build\n",
        "```\n",
    ));
    std::fs::write(docs_dir().join("GAS-RESULTS-V1.md"), md)?;
    println!("worst 300-move run with 100 graves: {worst_300_graves:.3} Tgas ({})", if pass { "PASS" } else { "FAIL" });
    assert!(pass, "300 moves with 100 graves cost {worst_300_graves:.3} Tgas > 10");
    check_counters(&env).await
}
