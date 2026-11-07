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
Optional (defaults): `RPC_URL` (`https://test.rpc.fastnear.com`), `PORT` (8787), `HOST` (0.0.0.0), `ALLOWED_ORIGIN` (`*`), `TRUST_PROXY_HOPS` (0), `SIGNUP_FUNDING_NEAR` (0.2), `RATE_RELAY_SENDER_MAX`/`_WINDOW_S` (30 / 60), `RATE_RELAY_IP_MAX`/`_WINDOW_S` (120 / 60), `RATE_SIGNUP_IP_MAX`/`_WINDOW_S` (5 / 3600), `RATE_SIGNUP_GLOBAL_MAX`/`_WINDOW_S` (200 / 86400).

## API

- `GET /health` -> `200 {ok, parent, world, blockHeight, balanceNear}`; `503` if the RPC is unreachable.
- `POST /signup {name, publicKey}` -> `200 {accountId, created, txHash?}` (idempotent for the same name and key); `400 bad_name|bad_public_key|bad_json`, `409 name_taken`, `429`, `502`.
- `POST /relay {signedDelegate}` (base64 borsh `SignedDelegate`) -> `200 {ok, txHash, method, resultBase64?}`; `400 bad_delegate|delegate_too_large|bad_json`, `403 wrong_sender|wrong_receiver|bad_actions|forbidden_action|forbidden_method|deposit_not_allowed|gas_too_high|gas_zero|args_too_large|bad_signature`, `422 call_failed` (the contract call failed; `txHash` included), `429` (+`Retry-After`), `502 chain_error`.

What the relayer pays for: sender is `<name>.<parent>` (a direct sub-account, valid player name), receiver is `WORLD_CONTRACT_ID`, exactly one FunctionCall to `start_dive` or `settle_dive`, deposit 0, gas at most 100 Tgas, arguments at most 8 KiB, valid ed25519 signature. Names `world`, `world-demo`, `lastserver`, `relayer`, `studio`, `admin`, `root`, `system` are reserved.

`scripts/e2e_testnet.mjs` (repo root) runs sign-up, start and settle through the relayer against a live testnet and the world contract.
