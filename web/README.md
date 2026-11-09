# Last Server web client

Static page (Vite + TypeScript). It runs the same Rust rules the contract runs (`crates/ls-wasm`, compiled to wasm), reads the world from the NEAR testnet through public RPC, and sends `start_dive` / `settle_dive` as the player. No wallet window, no seed phrase, no gas prompt: the browser makes an ed25519 key per player name and keeps it in `localStorage`.

## Run locally

```
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-bindgen-cli --version 0.2.130   # must match the wasm-bindgen crate in Cargo.lock
cd web
npm ci
npm run wasm      # builds crates/ls-wasm and writes web/src/wasm/ (not committed)
npm run dev       # http://localhost:5173
```

`npm run build` runs the wasm step, the type check and `vite build`; the site is `web/dist`. `npm test` runs the unit tests (relayer client, call retry policy, config, name rules, wasm parity).

### Without a deployed contract

`relayer/dev/mock-world.mjs` is a local stand-in for testnet, the world contract and the studio account (it runs the real relayer, `NearChain` included, against a fake RPC that answers in testnet's shapes, and the same wasm rules):

```
cd relayer && npm ci && node dev/mock-world.mjs      # RPC :3030, relayer :8787
cd web && VITE_RPC_URL=http://localhost:3030 VITE_RELAYER_URL=http://localhost:8787 npm run dev
```

It cannot catch a contract bug; it exercises the UI, sign-up, the signed-delegate path and the direct-send fallback.

## Environment (all optional, read at build time)

| Variable | Default | Meaning |
|---|---|---|
| `VITE_RPC_URL` | `https://test.rpc.fastnear.com` | NEAR JSON-RPC endpoint |
| `VITE_WORLD_CONTRACT_ID` | `world.lastserver.testnet` | world contract |
| `VITE_RELAYER_URL` | `http://localhost:8787` in `npm run dev`, empty in a build | relayer base URL; empty = players always send their own transactions |
| `VITE_PARENT_ACCOUNT_ID` | `lastserver.testnet` | studio account; players are `<name>.<parent>` |

## How a call is sent

1. The page builds a NEP-366 `SignedDelegate` with the player's key (100 Tgas, deposit 0, expires in about 2 minutes) and POSTs it to the relayer, which pays the gas.
2. If the relayer is unreachable, times out, answers 5xx or 429, the relayer may still have sent the call (`504 relay_unknown` says so outright). The page first reads `active_dive`: if the call already took effect (`start_dive`: a dive is open; `settle_dive`: this dive is closed) it carries on; only otherwise does it send the same call directly from the player's account with 100 Tgas (new accounts get 0.2 testnet NEAR at sign-up).
3. If the relayer refuses the call (policy 4xx) the page shows the reason and does not retry. If the contract refuses it, the page reads `active_dive` the same way first: a refusal caused by an earlier attempt that went through (`no active dive` after a lost reply) counts as done.

Only the way up keeps what you carry: on each depth it is the tile where the dive arrived (drawn as a blue arrow). Standing on it, `U` or the `Climb to the surface` button sends `MOVE_SURFACE` (byte 6) and the dive is recorded as `returned`. A log that ends anywhere else (out of moves, or given up) is a `quit`, which the rules count as a death: grave where the player stands, death toll. The result screen tells the three endings apart (returned / killed / gave up).

A dive in progress is saved in `localStorage` (move log plus the state it started from), so closing the tab does not lose it; `Continue the dive` replays it. `Give up the dive (counts as death)` on the surface asks for confirmation, then records the open dive with the moves this browser has (none: an empty log, so the grave is at the dive's entrance).

## Notes

- Keys in `localStorage` are readable by any script on the page origin. The accounts hold testnet NEAR only and the page loads no third-party scripts.
- New names need the relayer (it creates the account). A browser that already holds a key keeps playing when the studio backend is off.
