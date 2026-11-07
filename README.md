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

## Network

NEAR testnet only. No mainnet, no real money.

## Our other NEAR Hackathon project

The same team submits one more, independent project: **Context Firewall** (track: Privacy & Private AI). Its repository link will be added here when it is public.

## License

MIT, see [LICENSE](LICENSE).
