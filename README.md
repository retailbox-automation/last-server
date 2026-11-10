# Last Server

A small dungeon-diving game whose world lives in a NEAR contract, so it keeps running after the studio turns its server off.

Built at NEAR HACKATHON 2026 (track: Games With Real Economies). Work in progress: this repository was created on 2026-11-07, inside the hackathon window, and its full history is the build log.

## What it is

- One dive takes a few minutes. If your character dies, everything it carried stays at that depth as a grave with your name on it. The next player who reaches that depth can take it.
- Items are NEP-245 multi-tokens. Graves and dive results are contract state.
- The contract settles each dive by re-running the same deterministic game code (one Rust crate, compiled both into the browser and into the contract) on the day's seed and the player's move log. The client cannot invent loot.
- The web client is a static page that reads the chain through public RPC. The studio backend (a relayer) only creates new player accounts and pays gas. When it does not answer, a browser that already holds a player key sends the same call from the player's own account, so the game keeps working; only new names wait for the relayer.

## Check it in five minutes

| Claim | Where to see it | How to check |
|---|---|---|
| The game runs on NEAR testnet, not on our server | contract `world.lastserver.testnet` | Play at https://lastserver.retailbox-automation.com, then open the "Dive settled" link on the result screen |
| The contract runs exactly the rules in this repo | `core_version().rules_sha256` | Step 2 of Try it: the hash of `crates/ls-core/src/rules.rs` equals the one the contract returns |
| Loot is decided by the contract, not by the client | `settle_dive` replays the move log on the day's seed | `scripts/e2e_testnet.mjs` compares every contract result with the browser's wasm replay (37/37 on 2026-11-09) |
| A dead player's items become a grave someone else can take | events `grave_created`, `grave_looted` | The loot transaction from the testnet run: [3qnAD5…](https://testnet.nearblocks.io/txns/3qnAD5gx85TYb95BBNrKA6y8WqUf2wViV5DWPJGxhpeD) |
| Replaying a dive on chain is cheap | measured gas | [docs/spike/GAS-TESTNET.md](docs/spike/GAS-TESTNET.md), [docs/spike/GAS-RESULTS-V1.md](docs/spike/GAS-RESULTS-V1.md) |
| The game keeps working with the studio backend off | the page sends the call from the player's own account | On 2026-11-10 we stopped the demo relayer and played a dive from a browser that already had a key: [start](https://testnet.nearblocks.io/txns/88B5T5kVuwJMAwgNAxe8gxt8KWiXNzWrKchMdut5qDRk) and [settle](https://testnet.nearblocks.io/txns/FQ7eQMi4i5LTRK6Hf1CG28rKbs6duyEc3gbg52DPg9Re) are signed by the player, not by the studio |
| No wallet window for the player | the page makes a key per name | Play: you type a name, nothing asks you to sign or pay |

## Status

Core v1 (2026-11-09): rules crate, world contract, relayer and web client are built and pass their tests against a local sandbox and a mock chain. Verified end-to-end on NEAR testnet: 37/37 checks pass (contract `world.lastserver.testnet`, 2026-11-09). The run, [scripts/e2e_testnet.mjs](scripts/e2e_testnet.mjs), signs up two players through the relayer and checks that the relayer refuses a wrong method, a wrong receiver, a deposit, gas over 100 Tgas and a long-lived delegate. The first player dies and leaves a grave; the second loots it and climbs out. Each contract result equals the web client's replay of the same move log. The loot transaction: [3qnAD5…](https://testnet.nearblocks.io/txns/3qnAD5gx85TYb95BBNrKA6y8WqUf2wViV5DWPJGxhpeD).

Hosted demo (2026-11-10): the web client at https://lastserver.retailbox-automation.com and the relayer at https://lastserver-relay.retailbox-automation.com, both on NEAR testnet against `world.lastserver.testnet`. The relayer creates at most 15 new players a day (5 per IP per hour) and gives each 0.15 testnet NEAR, enough to keep playing when the relayer is off.

## Try it

Needs Rust with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` 0.2.130 (`cargo install --locked wasm-bindgen-cli --version 0.2.130`) and Node 22.22.2 or newer.

1. Run the rules tests: `cargo test -p ls-core --release`. You see `test result: ok` with `0 failed`.
2. Check that the contract on testnet runs these rules. `shasum -a 256 crates/ls-core/src/rules.rs` prints `15e75b62a854bc2b87c310d975e3a3b1fd655afe25620e9908d83fe050bab457`. Then ask the contract:

   ```sh
   curl -s https://test.rpc.fastnear.com -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","id":1,"method":"query","params":{"request_type":"call_function","finality":"final","account_id":"world.lastserver.testnet","method_name":"core_version","args_base64":"e30="}}' \
     | python3 -c 'import sys, json; print(bytes(json.load(sys.stdin)["result"]["result"]).decode())'
   ```

   You see JSON whose `rules_sha256` is the same hash.
3. Build and test the relayer and the web client: `cd relayer && npm ci && npm test`, then `cd ../web && npm ci && npm run build && npm test`. Each test summary ends with `ℹ fail 0`; the build writes the site to `web/dist`.
4. Play at https://lastserver.retailbox-automation.com: type a name and press Play. You see "Today is day N" and "Graves in the world", read straight from the contract. Choose what to carry and press Dive. Move with the arrow keys or WASD, walk into a monster to attack, take the green stairs down. To keep what you carry, go back to the blue arrow where you arrived on the current depth and press `U`. You see "Dive recorded." and, under "On the chain", links to the "Dive started" and "Dive settled" transactions on the testnet explorer. Die instead, and your items stay on that depth as a grave with your name on it for the next player.
5. The studio can go away. A browser that already holds a player key keeps playing when the relayer does not answer: the page sends the same call from the player's own account, and the result screen says "sent directly with your own key (the relayer was not available)". New names wait for the relayer, because it creates the accounts.

The full testnet run needs a funded testnet account to act as the studio: it creates the two players as its sub-accounts and pays their gas (a little testnet NEAR; the players are deleted at the end). We ran it with our studio account `lastserver.testnet`:

```sh
cd relayer && npm ci && cd ../web && npm ci && npm run build && cd ..
LASTSERVER_ACCOUNT_ID=<studio>.testnet LASTSERVER_SECRET_KEY=ed25519:... \
WORLD_CONTRACT_ID=world.lastserver.testnet RPC_URL=https://test.rpc.fastnear.com node scripts/e2e_testnet.mjs
```

It ends with `ALL CHECKS PASSED`.

## Platform findings (measured, day 1)

1. **Re-running the game rules inside a NEAR contract is cheap.** `settle_dive` replays a whole dive (map generation per depth, monster moves, combat, items) from the seed and the move log. On live testnet (protocol 88, 2026-11-07) a 300-move dive burned 3.95–4.98 Tgas, a 3000-move dive 14.2 Tgas, against a per-call limit of 1000 Tgas. The local sandbox gives the same numbers within 5 % (longer account names cost a little more storage on testnet). Every call's result equals the native run of the same crate. Details: [docs/spike/GAS-TESTNET.md](docs/spike/GAS-TESTNET.md) (with transaction links), [docs/spike/GAS-RESULTS.md](docs/spike/GAS-RESULTS.md) (sandbox, 33 runs, fit: 2.09 Tgas + 0.0025 Tgas per move + 0.28 Tgas per generated map).
2. **State you load on every call costs more than the computation.** In the day-1 version every dive loads all graves: each grave adds about 0.17 Tgas to every dive, so core v1 stores graves per depth and caps them (see Rules and contract below).
3. **Attached gas is paid up front.** On testnet the signer must hold prepaid gas × gas price before the call: attaching 1000 Tgas needed 1.0009 NEAR on the account (unused gas is refunded). Player accounts attach 100 Tgas.
4. **The default testnet RPC keeps about 35 hours of history** (211 112 blocks on 2026-11-07); older blocks and transactions answer `UNKNOWN_BLOCK` there and are served by `archival-rpc.testnet.near.org`. The same endpoint answered `429 TooManyRequests` during a burst of test transactions. Links to transactions in this README point to an explorer; game statistics are kept as counters in the contract.
5. **Test tooling note:** `near-workspaces` 0.24 `total_gas_burnt` does not include the final gas-refund receipt (about 0.22 Tgas per call on testnet, visible through the archival RPC).

## Rules and contract (core v1)

- **All numbers of the game are in one file:** [crates/ls-core/src/rules.rs](crates/ls-core/src/rules.rs). Its sha256 is reported by the contract (`core_version().rules_sha256`) and by the client, so you can check both run the same rules: `shasum -a 256 crates/ls-core/src/rules.rs`.
- **Items** (NEP-245 token ids): `gold`, `blade` (wears out inside a dive, a broken blade is burned), `plate`, `potion` (drunk and burned), `blue_key` (drops from depth 5, opens the one vault at depth 7 and is used up), `gem` (only in that vault).
- **A dive is two calls.** `start_dive(loadout)` locks what you carry and returns a ticket with the day's seed, chosen by the contract. `settle_dive(dive_id, moves)` re-runs the rules on that seed and your move log (at most 4000 moves) and applies the result: finds are minted, graves you emptied move to you (at most 50 per dive; the rest stay where they are), and what you carried is banked only if the log ends with a return to the surface: walk back to where you came down to the current depth and take the way up (move byte 6). You can surface from the arrival tile of the current depth; leaving it to loot is the risk. A log that ends anywhere else, an empty log included, counts as a death where it ends: what you carried becomes your grave. A dive left open past its day can be closed by anyone with `expire_dive(account_id)` once 4 hours have passed since it started; it counts as a death too, with the same death toll.
- **Graves** are stored per depth, at most 10 per depth (the oldest crumbles and its items burn), and a dive reads only the graves of the depths it enters, so its gas grows with the graves on those depths and with the graves it loots, not with every grave on the server: [docs/spike/GAS-RESULTS-V1.md](docs/spike/GAS-RESULTS-V1.md).
- **NEP-245 is a subset, written by hand from the spec:** `mt_transfer`, `mt_batch_transfer` (exactly 1 yoctoNEAR), `mt_balance_of`, `mt_batch_balance_of`, `mt_supply`, `mt_batch_supply`, `mt_token`, and the `mt_mint` / `mt_transfer` / `mt_burn` events. Not implemented: `mt_transfer_call` / `mt_resolve_transfer` (no other contract in this game receives items, and the cross-contract refund path is the riskiest part of the standard to hand-write in a week), approvals (no marketplace: items have use, not a price) and NEP-145 storage deposits (the world contract pays for its own state on testnet, so items can be transferred only to accounts that have started a dive). Items carried in an active dive cannot be transferred.
- **Game events** use NEP-297 with `standard: "lastserver"`, `version: "1.0.0"`: `dive_started`, `dive_settled`, `grave_created`, `grave_looted`, `grave_crumbled`, `dive_expired`.

## Limits

- Storage for the player and balance records of new accounts is paid by the world contract (`start_dive` comes through the relayer with zero deposit). This is deliberate on testnet; before mainnet the first `start_dive` of an account would take a one-time NEP-145 storage deposit.
- The demo relayer pays the gas of relayed calls: it stops at a floor of the studio balance and relays at most 1000 calls per 24 hours in total. When that budget is spent (anyone can spend it with calls that fail), the page sends each call from the player's own account, which costs the player a little testnet NEAR; the game keeps working.
- Gas: the worst measured 300-move dive with 100 graves on depths 1 to 10 costs 9.41 Tgas ([docs/spike/GAS-RESULTS-V1.md](docs/spike/GAS-RESULTS-V1.md)). Looting drives the cost (about 0.10 Tgas per looted grave, 0.02 Tgas per grave read, estimated from those rows), so a dive that loots close to the per-dive cap of 50 graves would cost about 11.5 Tgas (estimate, not measured). The client attaches 100 Tgas per call.

## Libraries and tools

Rust: near-sdk 5.30.1 (contract), wasm-bindgen 0.2.130 (browser build of the rules), serde and serde_json, sha2 (build script), near-workspaces 0.24.0 with tokio and base64 (sandbox tests and gas benchmark). JavaScript: near-api-js 7.4.0 and borsh 2.0.0 (relayer), near-api-js 7.4.0 and Vite 8.3.5 (web), TypeScript 7.0.2, Node's built-in test runner. All code in this repository was written during the hackathon window; no templates or own pre-hackathon libraries.

Parts of the code and docs were written with AI coding assistants and reviewed and tested by the team.

## Network

NEAR testnet only. No mainnet, no real money.

## Our other NEAR Hackathon project

The same team submits one more, independent project: [Context Firewall](https://github.com/retailbox-automation/context-firewall) (track: Privacy & Private AI). It keeps card numbers, tax IDs, API keys and one-time codes out of what an AI agent's model sees. The two projects share no code.

## License

MIT, see [LICENSE](LICENSE).
