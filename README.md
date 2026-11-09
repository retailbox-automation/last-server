# Last Server

A small dungeon-diving game whose world lives in a NEAR contract, so it keeps running after the studio turns its server off.

Built at NEAR HACKATHON 2026 (track: Games With Real Economies). Work in progress: this repository was created on 2026-11-07, inside the hackathon window, and its full history is the build log.

## What it is

- One dive takes a few minutes. If your character dies, everything it carried stays at that depth as a grave with your name on it. The next player who reaches that depth can take it.
- Items are NEP-245 multi-tokens. Graves and dive results are contract state.
- The contract settles each dive by re-running the same deterministic game code (one Rust crate, compiled both into the browser and into the contract) on the day's seed and the player's move log. The client cannot invent loot.
- The web client is a static page that reads the chain through public RPC. The studio backend is only a convenience: in the demo it is switched off and the game keeps working.

## Status

Day 1 (2026-11-07): repository, design, gas spike. Nothing here is playable yet. This section will be replaced by "do X — see Y" steps as features land.

## Platform findings (measured, day 1)

1. **Re-running the game rules inside a NEAR contract is cheap.** `settle_dive` replays a whole dive (map generation per depth, monster moves, combat, items) from the seed and the move log. On live testnet (protocol 88, 2026-11-07) a 300-move dive burned 3.95–4.98 Tgas, a 3000-move dive 14.2 Tgas, against a per-call limit of 1000 Tgas. The local sandbox gives the same numbers within 5 % (longer account names cost a little more storage on testnet). Every call's result equals the native run of the same crate. Details: [docs/spike/GAS-TESTNET.md](docs/spike/GAS-TESTNET.md) (with transaction links), [docs/spike/GAS-RESULTS.md](docs/spike/GAS-RESULTS.md) (sandbox, 33 runs, fit: 2.09 Tgas + 0.0025 Tgas per move + 0.28 Tgas per generated map).
2. **State you load on every call costs more than the computation.** In the day-1 version every dive loads all graves: each grave adds about 0.17 Tgas to every dive, so graves will be stored per depth and capped.
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

- Storage for the player and balance records of new accounts is paid by the world contract (`start_dive` comes through the relayer with zero deposit). This is deliberate on testnet.

## Network

NEAR testnet only. No mainnet, no real money.

## Our other NEAR Hackathon project

The same team submits one more, independent project: **Context Firewall** (track: Privacy & Private AI). Its repository link will be added here when it is public.

## License

MIT, see [LICENSE](LICENSE).
