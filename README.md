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

## Network

NEAR testnet only. No mainnet, no real money.

## Our other NEAR Hackathon project

The same team submits one more, independent project: **Context Firewall** (track: Privacy & Private AI). Its repository link will be added here when it is public.

## License

MIT, see [LICENSE](LICENSE).
