//! Local-sandbox checks for the Last Server contract.
//!
//! Build the wasm first:
//!   cargo build --target wasm32-unknown-unknown --release -p ls-contract
//! then: cargo test -p spike --release -- --test-threads=1 --nocapture
//!
//! `bench_settle_dive_gas` writes docs/spike/GAS-RESULTS.json and docs/spike/GAS-RESULTS.md.

use base64::Engine;
use ls_core::bot::{generate_moves, Style, BOT_MAX_DEPTH};
use ls_core::{run_dive, DiveEntry, DiveOutcome};
use near_workspaces::network::Sandbox;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, Contract, Worker};
use serde_json::{json, Value};
use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PREPAID_TGAS: u64 = 1000;
const TOKENS: [&str; 4] = ["gold", "blade", "plate", "gem"];

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

struct Env {
    worker: Worker<Sandbox>,
    contract: Contract,
    wasm_len: usize,
}

async fn setup() -> Result<Env, Box<dyn std::error::Error>> {
    let path = wasm_path();
    let wasm = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let worker = near_workspaces::sandbox().await?;
    let contract = worker.dev_deploy(&wasm).await?;
    contract.call("new").transact().await?.into_result()?;
    Ok(Env { worker, contract, wasm_len: wasm.len() })
}

async fn new_account(env: &Env, name: &str) -> Result<Account, Box<dyn std::error::Error>> {
    let root = env.worker.root_account()?;
    let acc = root
        .create_subaccount(name)
        .initial_balance(NearToken::from_near(20))
        .transact()
        .await?
        .into_result()?;
    Ok(acc)
}

async fn dive_entry(env: &Env, who: &Account) -> Result<DiveEntry, Box<dyn std::error::Error>> {
    Ok(env.contract.view("dive_entry").args_json(json!({ "account_id": who.id() })).await?.json()?)
}

async fn balance(env: &Env, who: &near_workspaces::AccountId, token: &str) -> Result<u128, Box<dyn std::error::Error>> {
    let v: String = env
        .contract
        .view("mt_balance_of")
        .args_json(json!({ "account_id": who, "token_id": token }))
        .await?
        .json()?;
    Ok(v.parse()?)
}

async fn balances(env: &Env, who: &near_workspaces::AccountId) -> Result<Vec<u128>, Box<dyn std::error::Error>> {
    let v: Vec<String> = env
        .contract
        .view("mt_batch_balance_of")
        .args_json(json!({ "account_id": who, "token_ids": TOKENS }))
        .await?
        .json()?;
    Ok(v.iter().map(|s| s.parse().unwrap()).collect())
}

fn settle_args(seed: u64, moves: &[u8]) -> Vec<u8> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(moves);
    serde_json::to_vec(&json!({ "seed": seed.to_string(), "moves": b64 })).unwrap()
}

/// Every `EVENT_JSON:` log line, parsed.
fn events(logs: &[&str]) -> Vec<Value> {
    logs.iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str(j).expect("event is JSON"))
        .collect()
}

fn assert_nep245(e: &Value, event: &str) {
    assert_eq!(e["standard"], "nep245", "{e}");
    assert_eq!(e["version"], "1.0.0", "{e}");
    assert_eq!(e["event"], event, "{e}");
    let data = e["data"].as_array().expect("data is an array");
    assert!(!data.is_empty());
    for d in data {
        let ids = d["token_ids"].as_array().expect("token_ids");
        let amounts = d["amounts"].as_array().expect("amounts");
        assert_eq!(ids.len(), amounts.len(), "{e}");
        for a in amounts {
            a.as_str().expect("amount is a string").parse::<u128>().expect("amount is base-10");
        }
        match event {
            "mt_mint" | "mt_burn" => assert!(d["owner_id"].is_string(), "{e}"),
            "mt_transfer" => {
                assert!(d["old_owner_id"].is_string(), "{e}");
                assert!(d["new_owner_id"].is_string(), "{e}");
            }
            _ => panic!("unexpected event {event}"),
        }
    }
}

struct Run {
    record: Value,
    outcome: DiveOutcome,
    total_gas: u64,
}

/// Calls settle_dive from `who` and checks the contract's outcome against the native rules.
async fn settle(env: &Env, who: &Account, seed: u64, moves: &[u8]) -> Result<(Run, ExecutionFinalResult), Box<dyn std::error::Error>> {
    let entry = dive_entry(env, who).await?;
    let native = run_dive(seed, &entry, moves);
    let args = settle_args(seed, moves);
    let args_len = args.len();
    let res = who
        .call(env.contract.id(), "settle_dive")
        .args(args)
        .gas(Gas::from_tgas(PREPAID_TGAS))
        .transact()
        .await?;
    let total_gas = res.total_gas_burnt.as_gas();
    let tx = res.outcome();
    let mut tokens_burnt = tx.tokens_burnt.as_yoctonear();
    let mut receipts = vec![];
    let mut fn_receipt_gas = None;
    for r in res.receipt_outcomes() {
        tokens_burnt += r.tokens_burnt.as_yoctonear();
        if fn_receipt_gas.is_none() && &r.executor_id == env.contract.id() {
            fn_receipt_gas = Some(r.gas_burnt.as_gas());
        }
        receipts.push(json!({
            "executor_id": r.executor_id,
            "gas_burnt": r.gas_burnt.as_gas(),
            "tokens_burnt_yocto": r.tokens_burnt.as_yoctonear().to_string(),
        }));
    }
    let tx_gas = tx.gas_burnt.as_gas();
    let ok = res.clone().into_result().map_err(|e| format!("settle_dive failed: {e:?}"))?;
    let outcome: DiveOutcome = ok.json()?;
    let equal = outcome == native;
    assert!(equal, "contract outcome differs from native run_dive\ncontract: {outcome:?}\nnative:   {native:?}");
    let record = json!({
        "account": who.id(),
        "seed": seed,
        "moves_in_log": moves.len(),
        "moves_applied": outcome.moves_applied,
        "start_depth": entry.start_depth,
        "final_depth": outcome.final_depth,
        "depths_visited": outcome.final_depth - entry.start_depth.max(1) + 1,
        "died": outcome.died,
        "kills": outcome.kills,
        "picked": outcome.picked,
        "inventory_in": entry.inventory,
        "graves_in": entry.graves.len(),
        "args_bytes": args_len,
        "total_gas_burnt": total_gas,
        "tx_conversion_gas_burnt": tx_gas,
        "function_call_receipt_gas_burnt": fn_receipt_gas,
        "receipts": receipts,
        "tokens_burnt_yocto": tokens_burnt.to_string(),
        "tokens_burnt_near": tokens_burnt as f64 / 1e24,
        "deterministic_equal": equal,
    });
    Ok((Run { record, outcome, total_gas }, res))
}

/// Ordinary least squares for y = c0 + c1 * x1 + c2 * x2 (normal equations, Gaussian elimination).
fn fit3(points: &[(f64, f64, f64)]) -> [f64; 3] {
    let mut m = [[0.0f64; 4]; 3];
    for &(x1, x2, y) in points {
        let row = [1.0, x1, x2];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += row[i] * row[j];
            }
            m[i][3] += row[i] * y;
        }
    }
    for col in 0..3 {
        let pivot = (col..3).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs())).unwrap();
        m.swap(col, pivot);
        for r in 0..3 {
            if r != col {
                let f = m[r][col] / m[col][col];
                for c in col..4 {
                    m[r][c] -= f * m[col][c];
                }
            }
        }
    }
    [m[0][3] / m[0][0], m[1][3] / m[1][1], m[2][3] / m[2][2]]
}

fn tgas(g: u64) -> f64 {
    g as f64 / 1e12
}

#[tokio::test]
async fn bench_settle_dive_gas() -> TestResult {
    let env = setup().await?;
    let status = env.worker.status().await?;
    let sizes = [50usize, 300, 1000, 3000];
    let seeds = [1u64, 2, 3];
    let mut runs = vec![];
    let mut summary = vec![];
    // (moves applied, maps generated = final depth from depth 1, total gas)
    let mut points: Vec<(f64, f64, f64)> = vec![];

    for (style, label) in [(Style::Survivor, "survivor"), (Style::Rusher, "rusher")] {
        for &n in &sizes {
            let mut gas = vec![];
            let mut depths = vec![];
            let mut applied = vec![];
            for &seed in &seeds {
                // A fresh account per dive: empty inventory, same starting point for every row.
                let who = new_account(&env, &format!("{label}{n}s{seed}")).await?;
                let entry = dive_entry(&env, &who).await?;
                let moves = generate_moves(seed, &entry, n, style);
                let (run, _) = settle(&env, &who, seed, &moves).await?;
                println!(
                    "{label} n={n} seed={seed}: applied={} depth={} died={} total={:.3} Tgas",
                    run.outcome.moves_applied,
                    run.outcome.final_depth,
                    run.outcome.died,
                    tgas(run.total_gas)
                );
                gas.push(run.total_gas);
                depths.push(run.outcome.final_depth);
                applied.push(run.outcome.moves_applied);
                points.push((
                    f64::from(run.outcome.moves_applied),
                    f64::from(run.outcome.final_depth),
                    run.total_gas as f64,
                ));
                let mut rec = run.record;
                rec["style"] = json!(label);
                rec["n"] = json!(n);
                runs.push(rec);
            }
            let mut sorted = gas.clone();
            sorted.sort_unstable();
            summary.push(json!({
                "style": label,
                "n": n,
                "moves_applied": applied,
                "final_depths": depths,
                "min_tgas": tgas(sorted[0]),
                "median_tgas": tgas(sorted[1]),
                "max_tgas": tgas(sorted[2]),
                "max_pct_of_1000_tgas": tgas(sorted[2]) / 10.0,
                "deterministic_equal_all": true,
            }));
        }
    }

    // Least squares over all runs: gas = fixed + per_move * moves + per_map * maps.
    let [fixed, per_move, per_map] = fit3(&points);
    let limit = PREPAID_TGAS as f64 * 1e12;
    let moves_per_map_survivor = 3000.0 / 15.0;
    let moves_per_map_rusher = 300.0 / 7.0;
    let max_moves_survivor = (limit - fixed) / (per_move + per_map / moves_per_map_survivor);
    let max_moves_rusher = (limit - fixed) / (per_move + per_map / moves_per_map_rusher);

    // Every dive loads all graves, so measure a 300-move dive with graves already on the server.
    // The pool is refilled to the target before each measured dive.
    let mut grave_rows = vec![];
    let mut made = 0u64;
    let nobody: near_workspaces::AccountId = "nobody.test.near".parse()?;
    // (A dive that runs deep also loots the graves on its path, so the pool grows slowly; 20 is
    // reachable in a few dozen grave dives.)
    for target in [5usize, 10, 20] {
        let mut gas = vec![];
        let mut graves_in = vec![];
        let mut looted = vec![];
        for &seed in &seeds {
            loop {
                let have: DiveEntry =
                    env.contract.view("dive_entry").args_json(json!({ "account_id": nobody })).await?.json()?;
                let have = have.graves.len();
                if have >= target {
                    break;
                }
                if made % 5 == 0 {
                    println!("graves in state: {have} (target {target}, grave dives so far {made})");
                }
                made += 1;
                if made > 150 {
                    return Err(format!("could not create {target} graves").into());
                }
                // A long run for the stairs picks things up and dies deep (depth ~7-16), leaving
                // a grave below where the measured 300-move dives go: this row measures the cost
                // of carrying graves in state, the looting cost shows up in the grave scene.
                let gseed = 1000 + made;
                let who = new_account(&env, &format!("dead{made}")).await?;
                let e = dive_entry(&env, &who).await?;
                let (_r, _x) = settle(&env, &who, gseed, &generate_moves(gseed, &e, 2000, Style::Rusher)).await?;
            }
            let who = new_account(&env, &format!("g{target}s{seed}")).await?;
            let entry = dive_entry(&env, &who).await?;
            let moves = generate_moves(seed, &entry, 300, Style::Survivor);
            let (run, _) = settle(&env, &who, seed, &moves).await?;
            println!(
                "graves={} n=300 seed={seed}: looted={} depth={} total={:.3} Tgas",
                entry.graves.len(),
                run.outcome.looted_graves.len(),
                run.outcome.final_depth,
                tgas(run.total_gas)
            );
            gas.push(run.total_gas);
            graves_in.push(entry.graves.len());
            looted.push(run.outcome.looted_graves.len());
            let mut rec = run.record;
            rec["style"] = json!("survivor-with-graves");
            rec["n"] = json!(300);
            runs.push(rec);
        }
        gas.sort_unstable();
        grave_rows.push(json!({
            "n": 300,
            "graves_in_state": graves_in,
            "graves_looted": looted,
            "min_tgas": tgas(gas[0]),
            "median_tgas": tgas(gas[1]),
            "max_tgas": tgas(gas[2]),
        }));
    }

    // Marginal cost of one grave in state: 20-grave median vs the no-grave 300-move median.
    let median_300 = summary
        .iter()
        .find(|s| s["style"] == "survivor" && s["n"] == 300)
        .map(|s| s["median_tgas"].as_f64().unwrap())
        .unwrap();
    let last = grave_rows.last().unwrap();
    let last_graves = last["graves_in_state"][1].as_u64().unwrap() as f64;
    let per_grave_tgas = (last["median_tgas"].as_f64().unwrap() - median_300) / last_graves;
    let graves_at_100_tgas = (100.0 - median_300) / per_grave_tgas;

    let worst_300 = summary
        .iter()
        .filter(|s| s["n"] == 300)
        .map(|s| s["max_tgas"].as_f64().unwrap())
        .fold(0.0, f64::max);
    let results = json!({
        "generated_by": "spike/tests/sandbox.rs bench_settle_dive_gas",
        "unix_time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        "sandbox": {
            "node_version": status.version.version,
            "node_build": status.version.build,
            "protocol_version": status.protocol_version,
            "latest_protocol_version": status.latest_protocol_version,
        },
        "wasm": { "file": "<CARGO_TARGET_DIR>/wasm32-unknown-unknown/release/ls_contract.wasm", "size_bytes": env.wasm_len },
        "prepaid_gas": Gas::from_tgas(PREPAID_TGAS).as_gas(),
        "bot_max_depth": BOT_MAX_DEPTH,
        "seeds": seeds,
        "summary": summary,
        "fit_all_runs": {
            "model": "gas = fixed + per_move * moves_applied + per_map * maps_generated",
            "gas_fixed": fixed,
            "gas_per_move": per_move,
            "gas_per_map": per_map,
            "max_moves_at_1000_tgas_survivor_pace": max_moves_survivor,
            "max_moves_at_1000_tgas_rusher_pace": max_moves_rusher,
        },
        "graves_scaling": grave_rows,
        "graves_marginal": {
            "tgas_per_grave_in_state": per_grave_tgas,
            "graves_in_state_for_300_moves_at_100_tgas": graves_at_100_tgas,
        },
        "threshold": {
            "n": 300,
            "limit_tgas": 100,
            "worst_tgas_any_style": worst_300,
            "pass": worst_300 <= 100.0,
        },
        "runs": runs,
    });
    std::fs::create_dir_all(docs_dir())?;
    std::fs::write(docs_dir().join("GAS-RESULTS.json"), serde_json::to_string_pretty(&results)?)?;

    let mut md = String::new();
    md.push_str("# settle_dive gas, local sandbox\n\n");
    md.push_str(&format!(
        "Generated by `spike/tests/sandbox.rs` (`bench_settle_dive_gas`). Sandbox node {} (protocol {}), wasm {} bytes, prepaid {} Tgas per call, seeds {:?}, one fresh account per dive. Raw numbers: `GAS-RESULTS.json`.\n\n",
        status.version.version, status.protocol_version, env.wasm_len, PREPAID_TGAS, seeds
    ));
    md.push_str("| log | moves | depth reached (per seed) | gas min / median / max, Tgas | max, % of 1000 Tgas | contract = native |\n");
    md.push_str("|---|---|---|---|---|---|\n");
    for s in results["summary"].as_array().unwrap() {
        md.push_str(&format!(
            "| {} | {} (applied {}) | {} | {:.2} / {:.2} / {:.2} | {:.2} % | yes |\n",
            s["style"].as_str().unwrap(),
            s["n"],
            s["moves_applied"],
            s["final_depths"],
            s["min_tgas"].as_f64().unwrap(),
            s["median_tgas"].as_f64().unwrap(),
            s["max_tgas"].as_f64().unwrap(),
            s["max_pct_of_1000_tgas"].as_f64().unwrap(),
        ));
    }
    md.push_str(&format!(
        "\nsurvivor = fights, rests, collects, takes stairs down to depth {BOT_MAX_DEPTH}; rusher = runs for the stairs every turn (most new maps per move; it dies early on long logs, see moves applied). Depth reached = maps generated, since every dive starts at depth 1.\n\nLeast-squares fit over all 24 runs: {:.3} Tgas fixed + {:.5} Tgas per move + {:.4} Tgas per new map. At 1000 Tgas that is about {:.0} moves at the survivor pace (1 map per 200 moves) or {:.0} at the rusher pace (1 map per ~43 moves), before other limits (4 MiB arguments, graves in state) matter.\n",
        fixed / 1e12,
        per_move / 1e12,
        per_map / 1e12,
        max_moves_survivor,
        max_moves_rusher,
    ));
    md.push_str("\n### Graves already on the server (every dive loads all of them)\n\n| moves | graves in state (per seed) | graves looted | gas min / median / max, Tgas |\n|---|---|---|---|\n");
    for g in results["graves_scaling"].as_array().unwrap() {
        md.push_str(&format!(
            "| 300 | {} | {} | {:.2} / {:.2} / {:.2} |\n",
            g["graves_in_state"],
            g["graves_looted"],
            g["min_tgas"].as_f64().unwrap(),
            g["median_tgas"].as_f64().unwrap(),
            g["max_tgas"].as_f64().unwrap(),
        ));
    }
    md.push_str(&format!(
        "\nEach grave in state adds about {per_grave_tgas:.3} Tgas to every dive (20-grave median vs no graves), so a 300-move dive reaches 100 Tgas at roughly {graves_at_100_tgas:.0} graves on the server: graves must be bucketed by depth and capped.\n"
    ));
    md.push_str(&format!(
        "\nThreshold: 300 moves must cost at most 100 Tgas. Worst 300-move run without graves: {:.2} Tgas, {}.\n",
        worst_300,
        if worst_300 <= 100.0 { "PASS" } else { "FAIL" }
    ));
    md.push_str(concat!(
        "\n## Reproduce\n\n",
        "```sh\n",
        "# toolchain: rustc with the wasm32-unknown-unknown target; .cargo/config.toml adds --cfg near for wasm\n",
        "cargo test -p ls-core --release\n",
        "cargo build --target wasm32-unknown-unknown --release -p ls-contract\n",
        "cargo test -p spike -- --test-threads=1 --nocapture   # downloads near-sandbox on first build\n",
        "```\n",
    ));
    std::fs::write(docs_dir().join("GAS-RESULTS.md"), md)?;
    println!(
        "worst 300-move run: {worst_300:.3} Tgas; fit {:.3} + {:.5}/move + {:.4}/map Tgas; ~{max_moves_survivor:.0} / {max_moves_rusher:.0} moves at limit",
        fixed / 1e12,
        per_move / 1e12,
        per_map / 1e12
    );
    Ok(())
}

#[tokio::test]
async fn grave_scene_alice_dies_bob_loots() -> TestResult {
    let env = setup().await?;
    let alice = new_account(&env, "alice").await?;
    let bob = new_account(&env, "bob").await?;
    let me = env.contract.id().clone();
    let seed = 42;

    // 1. Alice survives a dive and gets her finds minted (mt_mint).
    let entry = dive_entry(&env, &alice).await?;
    let moves = generate_moves(seed, &entry, 600, Style::Survivor);
    let (first, res) = settle(&env, &alice, seed, &moves).await?;
    assert!(!first.outcome.died);
    assert!(!first.outcome.picked.is_empty());
    let ev = events(&res.logs());
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_nep245(&ev[0], "mt_mint");
    assert_eq!(ev[0]["data"][0]["owner_id"], alice.id().as_str());
    let carried = balances(&env, alice.id()).await?;
    println!("alice after first dive: {carried:?} (gold, blade, plate, gem)");
    for (i, t) in TOKENS.iter().enumerate() {
        let picked = first.outcome.picked.iter().find(|p| ls_core::item_name(p.0) == Some(t)).map_or(0, |p| p.1);
        assert_eq!(carried[i], u128::from(picked), "{t}");
    }
    let supply: Option<String> = env.contract.view("mt_supply").args_json(json!({"token_id": "gold"})).await?.json()?;
    assert_eq!(supply.unwrap().parse::<u128>()?, carried[0]);

    // 2. Alice dives again and walks into the monsters: she dies, her items become a grave.
    let entry = dive_entry(&env, &alice).await?;
    let moves = generate_moves(seed, &entry, 600, Style::Doomed);
    let (second, res) = settle(&env, &alice, seed, &moves).await?;
    assert!(second.outcome.died);
    let ev = events(&res.logs());
    let names: Vec<&str> = ev.iter().map(|e| e["event"].as_str().unwrap()).collect();
    println!("alice death events: {names:?}");
    for e in &ev {
        assert_nep245(e, e["event"].as_str().unwrap());
    }
    let to_grave = ev.iter().find(|e| e["event"] == "mt_transfer").expect("transfer into grave");
    assert_eq!(to_grave["data"][0]["old_owner_id"], alice.id().as_str());
    assert_eq!(to_grave["data"][0]["new_owner_id"], me.as_str());
    if !second.outcome.burned.is_empty() {
        let burn = ev.iter().find(|e| e["event"] == "mt_burn").expect("death toll burn");
        assert_eq!(burn["data"][0]["owner_id"], alice.id().as_str());
    }
    assert_eq!(balances(&env, alice.id()).await?, vec![0, 0, 0, 0]);
    let depth = second.outcome.final_depth;
    let graves: Vec<Value> = env.contract.view("graves").args_json(json!({"depth": depth})).await?.json()?;
    println!("graves at depth {depth}: {graves:?}");
    assert_eq!(graves.len(), 1);
    assert_eq!(graves[0]["owner_id"], alice.id().as_str());
    let grave_items: Vec<(u32, u32)> = serde_json::from_value(graves[0]["items"].clone())?;
    assert_eq!(grave_items, second.outcome.dropped_into_grave);
    let held = balances(&env, &me).await?;
    for (i, t) in TOKENS.iter().enumerate() {
        let n = grave_items.iter().find(|g| ls_core::item_name(g.0) == Some(t)).map_or(0, |g| g.1);
        assert_eq!(held[i], u128::from(n), "contract holds {t}");
    }

    // 3. Bob dives the same seed, reaches the grave and loots it (mt_transfer contract -> bob).
    let entry = dive_entry(&env, &bob).await?;
    assert_eq!(entry.graves.len(), 1);
    let moves = generate_moves(seed, &entry, 600, Style::Looter);
    let bob_before = balances(&env, bob.id()).await?;
    let (third, res) = settle(&env, &bob, seed, &moves).await?;
    assert_eq!(third.outcome.looted_graves, vec![0]);
    assert!(!third.outcome.died);
    let ev = events(&res.logs());
    let loot = ev
        .iter()
        .find(|e| e["event"] == "mt_transfer" && e["data"][0]["old_owner_id"] == me.as_str())
        .expect("loot transfer");
    assert_nep245(loot, "mt_transfer");
    assert_eq!(loot["data"][0]["new_owner_id"], bob.id().as_str());
    assert_eq!(balances(&env, &me).await?, vec![0, 0, 0, 0], "grave emptied");
    let bob_after = balances(&env, bob.id()).await?;
    for (i, t) in TOKENS.iter().enumerate() {
        let looted = grave_items.iter().find(|g| ls_core::item_name(g.0) == Some(t)).map_or(0, |g| g.1);
        let picked = third.outcome.picked.iter().find(|p| ls_core::item_name(p.0) == Some(t)).map_or(0, |p| p.1);
        assert_eq!(bob_after[i], bob_before[i] + u128::from(looted) + u128::from(picked), "bob {t}");
    }
    let graves: Vec<Value> = env.contract.view("graves").args_json(json!({"depth": depth})).await?.json()?;
    assert!(graves.is_empty());
    println!("bob after looting: {bob_after:?}");

    // The contract reports the rules it runs.
    let v: String = env.contract.view("core_version").await?.json()?;
    assert!(v.starts_with("ls-core 0.1.0 rules-fnv1a64 "), "{v}");
    assert_eq!(v, format!("ls-core {} rules-fnv1a64 {}", ls_core::VERSION, ls_core::RULES_HASH));
    Ok(())
}

#[tokio::test]
async fn mt_transfer_needs_one_yocto() -> TestResult {
    let env = setup().await?;
    let alice = new_account(&env, "alice").await?;
    let bob = new_account(&env, "bob").await?;
    let seed = 7;
    let entry = dive_entry(&env, &alice).await?;
    let moves = generate_moves(seed, &entry, 400, Style::Survivor);
    let (_run, _res) = settle(&env, &alice, seed, &moves).await?;
    let gold = balance(&env, alice.id(), "gold").await?;
    assert!(gold >= 2, "alice found gold: {gold}");

    // Without the 1 yoctoNEAR deposit: fails, balances unchanged.
    let res = alice
        .call(env.contract.id(), "mt_transfer")
        .args_json(json!({ "receiver_id": bob.id(), "token_id": "gold", "amount": "1" }))
        .transact()
        .await?;
    assert!(res.is_failure());
    let err = format!("{:?}", res.into_result().unwrap_err());
    assert!(err.contains("Requires attached deposit of exactly 1 yoctoNEAR"), "{err}");
    assert_eq!(balance(&env, alice.id(), "gold").await?, gold);
    assert_eq!(balance(&env, bob.id(), "gold").await?, 0);

    // With 1 yoctoNEAR: succeeds and emits mt_transfer.
    let res = alice
        .call(env.contract.id(), "mt_transfer")
        .args_json(json!({ "receiver_id": bob.id(), "token_id": "gold", "amount": "2", "memo": "gift" }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    let ok = res.into_result()?;
    let ev = events(&ok.logs());
    assert_eq!(ev.len(), 1);
    assert_nep245(&ev[0], "mt_transfer");
    assert_eq!(ev[0]["data"][0]["old_owner_id"], alice.id().as_str());
    assert_eq!(ev[0]["data"][0]["new_owner_id"], bob.id().as_str());
    assert_eq!(ev[0]["data"][0]["token_ids"], json!(["gold"]));
    assert_eq!(ev[0]["data"][0]["amounts"], json!(["2"]));
    assert_eq!(ev[0]["data"][0]["memo"], "gift");
    assert!(ev[0]["data"][0].get("authorized_id").is_none());
    assert_eq!(balance(&env, alice.id(), "gold").await?, gold - 2);
    assert_eq!(balance(&env, bob.id(), "gold").await?, 2);

    // More than the balance: fails.
    let res = alice
        .call(env.contract.id(), "mt_transfer")
        .args_json(json!({ "receiver_id": bob.id(), "token_id": "gold", "amount": (gold + 1).to_string() }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    let err = format!("{:?}", res.into_result().unwrap_err());
    assert!(err.contains("insufficient balance"), "{err}");

    // Batch transfer with 1 yocto; mismatched lengths fail.
    let res = alice
        .call(env.contract.id(), "mt_batch_transfer")
        .args_json(json!({ "receiver_id": bob.id(), "token_ids": ["gold"], "amounts": ["1", "1"] }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?;
    let err = format!("{:?}", res.into_result().unwrap_err());
    assert!(err.contains("token_ids and amounts differ in length"), "{err}");
    let ok = alice
        .call(env.contract.id(), "mt_batch_transfer")
        .args_json(json!({ "receiver_id": bob.id(), "token_ids": ["gold"], "amounts": ["1"], "approvals": [null] }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?
        .into_result()?;
    assert_nep245(&events(&ok.logs())[0], "mt_transfer");
    assert_eq!(balance(&env, bob.id(), "gold").await?, 3);

    // Views: mt_token / mt_supply / mt_batch_supply for known and unknown ids.
    let tokens: Value = env.contract.view("mt_token").args_json(json!({"token_ids": ["gold", "sword"]})).await?.json()?;
    assert_eq!(tokens, json!([{ "token_id": "gold", "owner_id": null }, null]));
    let sup: Vec<Option<String>> =
        env.contract.view("mt_batch_supply").args_json(json!({"token_ids": ["gold", "sword"]})).await?.json()?;
    assert_eq!(sup[0].as_deref().map(|s| s.parse::<u128>().unwrap()), Some(gold));
    assert_eq!(sup[1], None);
    Ok(())
}
