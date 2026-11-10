# Last Server relayer

Small Node service: creates player accounts (`POST /signup`) and pays gas for their calls to the world contract (`POST /relay`, NEP-366 meta-transactions). It holds the studio key and nothing else. Needs Node >= 22.22.2 (a requirement of `near-api-js` 7).

```
cd relayer
npm ci
npm test                      # unit tests, no network
cp .env.example .env          # fill in; never commit it
set -a; . ./.env; set +a; npm start      # or: npm run build && npm run start:dist
```

## Environment

Required: `LASTSERVER_ACCOUNT_ID`, `LASTSERVER_SECRET_KEY` (`ed25519:...`), `WORLD_CONTRACT_ID`.
Optional (defaults): `RPC_URL` (`https://test.rpc.fastnear.com`), `PORT` (8787), `HOST` (0.0.0.0), `ALLOWED_ORIGIN` (`*`), `TRUST_PROXY_HOPS` (0), `SIGNUP_FUNDING_NEAR` (0.2), `MIN_STUDIO_BALANCE_NEAR` (1), `RATE_RELAY_SENDER_MAX`/`_WINDOW_S` (30 / 60), `RATE_RELAY_IP_MAX`/`_WINDOW_S` (120 / 60), `RATE_RELAY_GLOBAL_MAX`/`_WINDOW_S` (1000 / 86400; all relays together), `RATE_SIGNUP_IP_MAX`/`_WINDOW_S` (5 / 3600), `RATE_SIGNUP_GLOBAL_MAX`/`_WINDOW_S` (20 / 86400), `RATE_LOOKUP_IP_MAX`/`_WINDOW_S` (30 / 60; `GET /health` and the name check of `POST /signup`).

Budget: each signup moves `SIGNUP_FUNDING_NEAR` from the studio account to the new player for good. Signup answers `503 studio_funds_low` once that would leave the studio below `MIN_STUDIO_BALANCE_NEAR`; relays keep working down to the floor itself, so existing players keep playing; below the floor `/relay` answers `503 studio_funds_low` too, and the web client sends the call from the player's own account. All relays together are capped by `RATE_RELAY_GLOBAL_MAX` per window, because the studio pays the gas of every relayed call, failed ones included. Every `*_WINDOW_S` must be at least 1; limits take plain decimal integers. 20 signups a day at 0.2 NEAR is 4 testnet NEAR a day: top the studio up from the faucet, or lower the limits.

### Behind a reverse proxy (Zeabur, nginx, a load balancer)

Rate limits key on the client IP. With `TRUST_PROXY_HOPS=0` (the default) the relayer uses the socket address, which behind a proxy is the proxy's own address: every player then shares one bucket, and the sixth new player in an hour gets `429`. Set `TRUST_PROXY_HOPS` to the number of proxies that append to `X-Forwarded-For` in front of the relayer (one platform proxy: `1`). Do not set it when the relayer is reachable directly, or clients can pick their own IP by sending the header. If the header arrives while `TRUST_PROXY_HOPS=0`, the relayer logs one `{"ev":"warning"}` line.

## API

- `GET /health` -> `200 {ok, parent, world, blockHeight, funds}` (`funds` is `ok`, or `low` when signup is refused for lack of studio NEAR; cached 5 s); `429`; `503` if the RPC is unreachable.
- `POST /signup {name, publicKey}` -> `200 {accountId, created, txHash?}` (idempotent for the same name and key); `400 bad_name|bad_public_key|bad_json`, `409 name_taken`, `429`, `502`, `503 studio_funds_low`.
- `POST /relay {signedDelegate}` (base64 borsh `SignedDelegate`) -> `200 {ok, txHash, method, resultBase64?}`; `400 bad_delegate|delegate_too_large|bad_json`, `403 wrong_sender|wrong_receiver|bad_actions|forbidden_action|forbidden_method|deposit_not_allowed|gas_too_high|gas_zero|args_too_large|bad_signature|delegate_expired|delegate_ttl_too_long`, `422 call_failed` (the contract call failed; `txHash` included), `429` (+`Retry-After`), `502 chain_error` (nothing was executed), `503 studio_funds_low` (nothing was executed), `504 relay_unknown` (the transaction was sent but its outcome is unknown; `txHash` included: look it up or read the contract state before sending the call again).

What the relayer pays for: sender is `<name>.<parent>` (a direct sub-account, valid player name), receiver is `WORLD_CONTRACT_ID`, exactly one FunctionCall to `start_dive` or `settle_dive`, deposit 0, gas at most 100 Tgas, arguments at most 8 KiB, valid ed25519 signature, and a `maxBlockHeight` between 5 and 300 blocks past the current block (the web signs with 100). Names `world`, `world-demo`, `lastserver`, `relayer`, `studio`, `admin`, `root`, `system` are reserved.

`scripts/e2e_testnet.mjs` (repo root) runs sign-up, start and settle through the relayer against a live testnet and the world contract.
