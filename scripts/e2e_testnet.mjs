#!/usr/bin/env node
// End-to-end check against a LIVE testnet world contract, through the real relayer code:
//   sign up a fresh player -> start_dive -> settle_dive (both as NEP-366 signed delegates) -> read the result back over RPC.
// Run by whoever holds the studio key (lane NEARLS); it spends a little testnet NEAR (account funding + gas).
//
//   cd relayer && npm ci && cd ..
//   LASTSERVER_ACCOUNT_ID=lastserver.testnet LASTSERVER_SECRET_KEY=ed25519:... \
//   WORLD_CONTRACT_ID=world.lastserver.testnet [RPC_URL=...] [RELAYER_URL=http://host:8787] node scripts/e2e_testnet.mjs
//
// Without RELAYER_URL the script starts the relayer in this process (same env vars). Exit code 0 = all checks passed.
// Expected contract API: SPEC-CORE-V1 section A (start_dive, settle_dive, active_dive, player, current_day).

const relayerDir = new URL('../relayer/', import.meta.url);
const near = await import(new URL('node_modules/near-api-js/lib/index.js', relayerDir).href);
const { Account, JsonRpcProvider, KeyPair, KeyPairSigner, actions, encodeSignedDelegate } = near;

const env = process.env;
const need = (k) => { if (!env[k]) { console.error(`missing env ${k}`); process.exit(2); } return env[k]; };
const parent = need('LASTSERVER_ACCOUNT_ID');
const world = need('WORLD_CONTRACT_ID');
const rpcUrl = env.RPC_URL || 'https://test.rpc.fastnear.com';
const provider = new JsonRpcProvider({ url: rpcUrl });

let failures = 0;
const check = (ok, label, extra = '') => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${extra ? '  ' + extra : ''}`); if (!ok) failures++; };

let base = env.RELAYER_URL;
let server = null;
if (!base) {
  need('LASTSERVER_SECRET_KEY');
  const { loadConfig } = await import(new URL('src/config.ts', relayerDir).href);
  const { NearChain } = await import(new URL('src/chain.ts', relayerDir).href);
  const { createApp } = await import(new URL('src/app.ts', relayerDir).href);
  const config = loadConfig({ ...env, RATE_SIGNUP_IP_MAX: '100' });
  server = createApp({ policy: config, chain: new NearChain(config), log: () => {} });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}`;
}

const post = async (path, body) => {
  const res = await fetch(base + path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  return { status: res.status, json: await res.json().catch(() => null) };
};
const view = (method, args = {}) => provider.callFunction({ contractId: world, method, args, blockQuery: { finality: 'optimistic' } });
const b64json = (s) => JSON.parse(Buffer.from(s, 'base64').toString('utf8'));
const explorer = (h) => `https://testnet.nearblocks.io/txns/${h}`;

try {
  const health = await (await fetch(base + '/health')).json();
  check(health.ok === true, 'relayer /health', `block ${health.blockHeight}, studio balance ${health.balanceNear} NEAR`);

  // 1. sign up
  const name = `e2e-${Date.now().toString(36)}`;
  const key = KeyPair.fromRandom('ed25519');
  const accountId = `${name}.${parent}`;
  const su = await post('/signup', { name, publicKey: key.getPublicKey().toString() });
  check(su.status === 200 && su.json.created === true, `signup ${accountId}`, su.json?.txHash ? explorer(su.json.txHash) : JSON.stringify(su.json));
  if (su.status !== 200) throw new Error('signup failed');
  const again = await post('/signup', { name, publicKey: key.getPublicKey().toString() });
  check(again.status === 200 && again.json.created === false, 'signup is idempotent for the same key');
  const taken = await post('/signup', { name, publicKey: KeyPair.fromRandom('ed25519').getPublicKey().toString() });
  check(taken.status === 409, 'signup with another key for the same name -> 409');

  const player = new Account(accountId, provider, new KeyPairSigner(key));
  const delegate = async (method, args, gas = 100_000_000_000_000n, receiver = world, deposit = 0n) => {
    const { signedDelegate } = await player.createSignedMetaTransaction({
      receiverId: receiver, actions: [actions.functionCall(method, args, gas, deposit)], blockHeightTtl: 100,
    });
    return Buffer.from(encodeSignedDelegate(signedDelegate)).toString('base64');
  };

  // 2. the relayer refuses what it must refuse (no chain traffic)
  for (const [label, d, code] of [
    ['wrong method', await delegate('mt_transfer', {}), 'forbidden_method'],
    ['wrong receiver', await delegate('start_dive', {}, 100_000_000_000_000n, 'other.testnet'), 'wrong_receiver'],
    ['deposit', await delegate('start_dive', {}, 100_000_000_000_000n, world, 1n), 'deposit_not_allowed'],
    ['gas over 100 Tgas', await delegate('start_dive', {}, 200_000_000_000_000n), 'gas_too_high'],
  ]) {
    const r = await post('/relay', { signedDelegate: d });
    check(r.status === 403 && r.json?.error?.code === code, `relay refuses ${label}`, `${r.status} ${r.json?.error?.code}`);
  }

  // 3. start_dive through the relayer
  const start = await post('/relay', { signedDelegate: await delegate('start_dive', { loadout: [] }) });
  check(start.status === 200, 'start_dive relayed', start.json?.txHash ? explorer(start.json.txHash) : JSON.stringify(start.json));
  if (start.status !== 200) throw new Error('start_dive failed');
  const ticket = start.json.resultBase64 ? b64json(start.json.resultBase64) : await view('active_dive', { account_id: accountId });
  check(ticket && ticket.dive_id !== undefined, 'contract returned a dive ticket', JSON.stringify(ticket));
  const active = await view('active_dive', { account_id: accountId });
  check(active && String(active.dive_id) === String(ticket.dive_id), 'active_dive view shows the ticket');

  // 4. settle_dive through the relayer, with a plain move log
  const moves = Buffer.from(Array.from({ length: 200 }, (_, i) => [3, 3, 1, 1, 2, 4, 0, 5][i % 8]));
  const settle = await post('/relay', { signedDelegate: await delegate('settle_dive', { dive_id: String(ticket.dive_id), moves: moves.toString('base64') }) });
  check(settle.status === 200, 'settle_dive relayed', settle.json?.txHash ? explorer(settle.json.txHash) : JSON.stringify(settle.json));
  const outcome = settle.json?.resultBase64 ? b64json(settle.json.resultBase64) : null;
  check(outcome !== null && typeof outcome.died === 'boolean', 'contract returned a dive outcome', JSON.stringify(outcome));

  // 5. read back over RPC
  const after = await view('active_dive', { account_id: accountId });
  check(after === null || after === undefined, 'no active dive after settle');
  const me = await view('player', { account_id: accountId });
  check(me && Number(me.dives) === 1, 'player view counts one dive', JSON.stringify(me));
  const replay = await post('/relay', { signedDelegate: await delegate('settle_dive', { dive_id: String(ticket.dive_id), moves: moves.toString('base64') }) });
  check(replay.status === 422, 'second settle of the same dive fails on the contract (422 call_failed)', `${replay.status} ${replay.json?.error?.message ?? ''}`.slice(0, 160));
} catch (e) {
  failures++;
  console.error('ABORTED:', e.message);
} finally {
  server?.close();
}
console.log(failures === 0 ? '\nALL CHECKS PASSED' : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
