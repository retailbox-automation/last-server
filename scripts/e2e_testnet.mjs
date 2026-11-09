#!/usr/bin/env node
// End-to-end check against a LIVE testnet world contract, through the real relayer code:
//   contract core_version == the web's wasm rules -> sign up player A -> start_dive -> settle_dive with a
//   move log that gets A killed holding loot (both as NEP-366 signed delegates) -> A's grave is on the
//   chain -> sign up player B -> B's dive walks to that grave, loots it, walks back to the way up and
//   climbs out (MOVE_SURFACE; a log that ends inside the dungeon counts as a death) -> read back over RPC.
// Run by whoever holds the studio key (lane NEARLS); it spends a little testnet NEAR (2 accounts x
// SIGNUP_FUNDING_NEAR + gas). The move logs come from scripts/grave_bot.mjs on the web's wasm rules; if
// they find no plan on today's map, the script says so (TODO) and runs a dive that climbs straight out.
//
//   cd relayer && npm ci && cd ../web && npm ci && npm run build && cd ..      # build makes web/src/wasm
//   LASTSERVER_ACCOUNT_ID=lastserver.testnet LASTSERVER_SECRET_KEY=ed25519:... \
//   WORLD_CONTRACT_ID=world.lastserver.testnet [RPC_URL=...] [RELAYER_URL=http://host:8787] node scripts/e2e_testnet.mjs
//
// Without RELAYER_URL the script starts the relayer in this process (same env vars). Exit code 0 = all checks passed.
// Expected contract API: SPEC-CORE-V1 section A (start_dive, settle_dive, active_dive, player, current_day).

import { readFileSync } from 'node:fs';
import { SURFACE_NOW, graveFor, lootGrave } from './grave_bot.mjs';

const relayerDir = new URL('../relayer/', import.meta.url);
const near = await import(new URL('node_modules/near-api-js/lib/index.js', relayerDir).href);
const { Account, JsonRpcProvider, KeyPair, KeyPairSigner, actions, encodeSignedDelegate } = near;
const { AccessKeyDoesNotExistError, AccountDoesNotExistError } = await import(new URL('node_modules/near-api-js/lib/rpc-errors/index.js', relayerDir).href);
const missingKey = (e) => e instanceof AccessKeyDoesNotExistError || e instanceof AccountDoesNotExistError;
/** Public keys are not secret, but they are noise in a log: show the shape, not the value. */
const mask = (text) => String(text).replace(/ed25519:[1-9A-HJ-NP-Za-km-z]+/g, 'ed25519:<masked>');

const env = process.env;
const need = (k) => { if (!env[k]) { console.error(`missing env ${k}`); process.exit(2); } return env[k]; };
const parent = need('LASTSERVER_ACCOUNT_ID');
const world = need('WORLD_CONTRACT_ID');
const rpcUrl = env.RPC_URL || 'https://test.rpc.fastnear.com';
const provider = new JsonRpcProvider({ url: rpcUrl });

let failures = 0;
const check = (ok, label, extra = '') => { console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${extra ? '  ' + extra : ''}`); if (!ok) failures++; };
const todos = [];
const todo = (label) => { console.log(`TODO  ${label}`); todos.push(label); };

// The web's rules (wasm built by `npm run build` in web/): hashes for core_version and the move-log bots.
const wasmDir = new URL('../web/src/wasm/', import.meta.url);
let w = null;
let wasmError = '';
try {
  w = await import(new URL('ls_wasm.js', wasmDir).href);
  w.initSync({ module: readFileSync(new URL('ls_wasm_bg.wasm', wasmDir)) });
} catch (e) {
  w = null;
  wasmError = e.message;
}

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

const players = []; // accounts this run created; deleted at the end so their NEAR returns to the studio

/**
 * A key added a moment ago can be unknown to the RPC node that answers next (seen 2026-11-09 on
 * test.rpc.fastnear.com: signing right after signup failed with "Access key ... does not exist").
 * Wait until it shows at optimistic, then final finality: every 0.75 s, up to 15 s.
 */
async function waitForKey(accountId, publicKey) {
  const deadline = Date.now() + 15_000;
  for (const finality of ['optimistic', 'final']) {
    for (;;) {
      try {
        await provider.viewAccessKey({ accountId, publicKey, finalityQuery: { finality } });
        break;
      } catch (e) {
        if (Date.now() >= deadline) throw new Error(`the key of ${accountId} is not visible at ${finality} finality after 15 s: ${mask(e.message)}`);
        await new Promise((r) => setTimeout(r, 750));
      }
    }
  }
}

/** Signs up a fresh player through the relayer; returns { accountId, delegate(method, args, ...) }. */
async function newPlayer(prefix) {
  const name = `${prefix}-${Date.now().toString(36)}`;
  const key = KeyPair.fromRandom('ed25519');
  const accountId = `${name}.${parent}`;
  const su = await post('/signup', { name, publicKey: key.getPublicKey().toString() });
  check(su.status === 200 && su.json.created === true, `signup ${accountId}`, su.json?.txHash ? explorer(su.json.txHash) : JSON.stringify(su.json));
  if (su.status !== 200) throw new Error('signup failed');
  const player = new Account(accountId, provider, new KeyPairSigner(key));
  players.push(player);
  const t0 = Date.now();
  await waitForKey(accountId, key.getPublicKey());
  check(true, `${accountId}: key visible to the RPC (optimistic and final)`, `${Date.now() - t0} ms`);
  const sign = (method, args, gas, receiver, deposit, blockHeightTtl) =>
    player.createSignedMetaTransaction({ receiverId: receiver, actions: [actions.functionCall(method, args, gas, deposit)], blockHeightTtl });
  const delegate = async (method, args, gas = 100_000_000_000_000n, receiver = world, deposit = 0n, blockHeightTtl = 100) => {
    let signed;
    try {
      signed = await sign(method, args, gas, receiver, deposit, blockHeightTtl);
    } catch (e) {
      if (!missingKey(e)) throw e;
      // Same race as above, on a different RPC node: wait once more, then one retry.
      await waitForKey(accountId, key.getPublicKey());
      signed = await sign(method, args, gas, receiver, deposit, blockHeightTtl);
    }
    return Buffer.from(encodeSignedDelegate(signed.signedDelegate)).toString('base64');
  };
  return { name, key, accountId, delegate };
}

/** start_dive with an empty loadout; returns the ticket and the entry the contract will replay. */
async function startDive(p, label) {
  const start = await post('/relay', { signedDelegate: await p.delegate('start_dive', { loadout: [] }) });
  check(start.status === 200, `${label}: start_dive relayed`, start.json?.txHash ? explorer(start.json.txHash) : JSON.stringify(start.json));
  if (start.status !== 200) throw new Error('start_dive failed');
  const ticket = start.json.resultBase64 ? b64json(start.json.resultBase64) : await view('active_dive', { account_id: p.accountId });
  check(ticket && ticket.dive_id !== undefined, `${label}: contract returned a dive ticket`, JSON.stringify(ticket));
  const entry = await view('dive_entry', { account_id: p.accountId });
  return { ticket, entry };
}

async function settleDive(p, ticket, moves, label) {
  const settle = await post('/relay', { signedDelegate: await p.delegate('settle_dive', { dive_id: String(ticket.dive_id), moves: Buffer.from(moves).toString('base64') }) });
  check(settle.status === 200, `${label}: settle_dive relayed`, settle.json?.txHash ? explorer(settle.json.txHash) : JSON.stringify(settle.json));
  const outcome = settle.json?.resultBase64 ? b64json(settle.json.resultBase64) : null;
  check(outcome !== null && typeof outcome.died === 'boolean', `${label}: contract returned a dive outcome`, JSON.stringify(outcome)?.slice(0, 200));
  return outcome;
}

/** What a player sees of a dive; equal on both sides = the web replays the dive like the contract. */
const digest = (o) => JSON.stringify([o.died, o.returned, o.quit, o.final_depth, o.picked, o.looted_graves, o.dropped_into_grave, o.burned]);

try {
  const health = await (await fetch(base + '/health')).json();
  check(health.ok === true && health.funds === 'ok', 'relayer /health (studio can fund signups)', `block ${health.blockHeight}, funds ${health.funds}`);

  // 0. the contract and the page run the same rules
  const cv = await view('core_version');
  if (w === null) {
    check(false, 'core_version matches the web wasm', `web/src/wasm not loadable (${wasmError}); run npm run build in web/`);
  } else {
    check(cv.rules_sha256 === w.rules_hash() && cv.core_sha256 === w.core_hash(), 'core_version matches the web wasm',
      `contract rules ${cv.rules_sha256?.slice(0, 12)} core ${cv.core_sha256?.slice(0, 12)}; web rules ${w.rules_hash().slice(0, 12)} core ${w.core_hash().slice(0, 12)}`);
  }

  // 1. sign up player A
  const a = await newPlayer('e2e');
  const { accountId, delegate } = a;
  const again = await post('/signup', { name: a.name, publicKey: a.key.getPublicKey().toString() });
  check(again.status === 200 && again.json.created === false, 'signup is idempotent for the same key');
  const taken = await post('/signup', { name: a.name, publicKey: KeyPair.fromRandom('ed25519').getPublicKey().toString() });
  check(taken.status === 409, 'signup with another key for the same name -> 409');

  // 2. the relayer refuses what it must refuse (no chain traffic)
  for (const [label, d, code] of [
    ['wrong method', await delegate('mt_transfer', {}), 'forbidden_method'],
    ['wrong receiver', await delegate('start_dive', {}, 100_000_000_000_000n, 'other.testnet'), 'wrong_receiver'],
    ['deposit', await delegate('start_dive', {}, 100_000_000_000_000n, world, 1n), 'deposit_not_allowed'],
    ['gas over 100 Tgas', await delegate('start_dive', {}, 200_000_000_000_000n), 'gas_too_high'],
    ['a delegate valid for 10000 blocks', await delegate('start_dive', { loadout: [] }, 100_000_000_000_000n, world, 0n, 10_000), 'delegate_ttl_too_long'],
  ]) {
    const r = await post('/relay', { signedDelegate: d });
    check(r.status === 403 && r.json?.error?.code === code, `relay refuses ${label}`, `${r.status} ${r.json?.error?.code}`);
  }

  // 3. start_dive through the relayer
  const { ticket, entry } = await startDive(a, 'A');
  const active = await view('active_dive', { account_id: accountId });
  check(active && String(active.dive_id) === String(ticket.dive_id), 'active_dive view shows the ticket');

  // 4. settle_dive through the relayer: a log that gets A killed holding loot (or one that climbs straight out)
  // Pick something up, then get killed or quit (a log ending inside = a death there), whichever is shorter.
  const plan = w ? graveFor(w, ticket.seed, entry) : null;
  if (!plan) todo(`grave path: no "pick up, then die or quit" plan on today's map for A (seed ${ticket.seed}); A climbs straight out instead, grave path not tested`);
  const moves = plan ? plan.moves : SURFACE_NOW;
  const outcome = await settleDive(a, ticket, moves, 'A');
  if (!plan) check(outcome?.returned === true && outcome.died === false, 'A climbed out on the way up: returned, not a death', JSON.stringify(outcome)?.slice(0, 160));
  if (plan && outcome) {
    check(digest(outcome) === digest(plan.outcome), 'A: contract outcome equals the web wasm replay of the same log',
      `contract ${digest(outcome)} web ${digest(plan.outcome)}`.slice(0, 300));
  }

  // 5. read back over RPC
  const after = await view('active_dive', { account_id: accountId });
  check(after === null || after === undefined, 'no active dive after settle');
  const me = await view('player', { account_id: accountId });
  check(me && Number(me.dives) === 1, 'player view counts one dive', JSON.stringify(me));
  const replay = await post('/relay', { signedDelegate: await delegate('settle_dive', { dive_id: String(ticket.dive_id), moves: Buffer.from(moves).toString('base64') }) });
  check(replay.status === 422, 'second settle of the same dive fails on the contract (422 call_failed)', `${replay.status} ${replay.json?.error?.message ?? ''}`.slice(0, 160));

  // 6. the grave path: A's grave is on the chain, player B walks to it and takes it
  if (plan && outcome?.died) {
    if (plan.outcome.quit) {
      check(outcome.quit === true && outcome.died === true && !outcome.returned && Array.isArray(outcome.death_pos), 'A died (quit): the log ended inside the dungeon', `death_pos ${JSON.stringify(outcome.death_pos)}`);
    } else {
      check(outcome.quit === false && outcome.returned === false, 'A was killed (not a quit)');
    }
    check(Number(me.deaths) === 1, 'player view counts the death', JSON.stringify(me));
    const here = await view('graves', { depth: outcome.final_depth });
    const grave = here.find((g) => g.owner_id === accountId && String(g.dive_id) === String(ticket.dive_id));
    check(grave !== undefined, `A's grave is listed at depth ${outcome.final_depth}`, JSON.stringify(grave ?? here.length));
    if (grave && outcome.quit) {
      check(grave.x === outcome.death_pos[0] && grave.y === outcome.death_pos[1], 'grave created at death_pos', `grave (${grave.x}, ${grave.y})`);
    }
    if (grave) {
      const b = await newPlayer('e2e-b');
      const bDive = await startDive(b, 'B');
      const route = lootGrave(w, bDive.ticket.seed, bDive.entry, grave.id);
      if (!route) {
        todo(`grave path: no route for B to grave ${grave.id} and back up (seed ${bDive.ticket.seed}; killed on the way, or 50 graves looted first); B climbs straight out instead`);
        await settleDive(b, bDive.ticket, SURFACE_NOW, 'B');
      } else {
        const bOut = await settleDive(b, bDive.ticket, route.moves, 'B');
        check(bOut && bOut.looted_graves.includes(grave.id) && bOut.returned && !bOut.died, `B looted grave ${grave.id} and climbed out`, JSON.stringify(bOut?.looted_graves));
        check(bOut && bOut.looted_graves.length <= 50, 'B looted at most GRAVES_LOOTED_PER_DIVE (50) graves', String(bOut?.looted_graves.length));
        check(bOut && digest(bOut) === digest(route.outcome), 'B: contract outcome equals the web wasm replay of the same log');
        // The RPC behind a load balancer can lag a block or two after the settle: poll before failing.
        let left = await view('graves', { depth: grave.depth });
        const t0 = Date.now();
        while (left.some((g) => g.id === grave.id) && Date.now() - t0 < 15000) {
          await new Promise((r) => setTimeout(r, 1000));
          left = await view('graves', { depth: grave.depth });
        }
        check(!left.some((g) => g.id === grave.id), 'the grave is gone from the graves view', `${Date.now() - t0} ms`);
        const stash = new Map((await view('player', { account_id: b.accountId })).stash.map(([t, n]) => [t, BigInt(n)]));
        check(grave.items.every(([t, n]) => (stash.get(t) ?? 0n) >= BigInt(n)), "B's stash holds what the grave held", [...stash].map(([t, n]) => `${t}:${n}`).join(' '));
      }
    }
  } else if (plan) {
    todo('grave path: A survived on the contract although the web run died; see the outcome comparison above');
  }
} catch (e) {
  failures++;
  console.error('ABORTED:', mask(e.message));
} finally {
  // Each signup moved SIGNUP_FUNDING_NEAR from the studio; give what is left back (E2E_KEEP_ACCOUNTS=1 keeps them).
  if (!env.E2E_KEEP_ACCOUNTS) {
    for (const p of players) {
      try {
        const r = await p.deleteAccount(parent);
        check(true, `deleted ${p.accountId}, NEAR back to ${parent}`, explorer(r.transaction_outcome.id));
      } catch (e) {
        check(false, `delete ${p.accountId}`, mask(e.message).slice(0, 160));
      }
    }
  }
  server?.close();
}
if (todos.length > 0) console.log(`\n${todos.length} TODO (not tested this run): ${todos.join(' | ')}`);
console.log(failures === 0 ? '\nALL CHECKS PASSED' : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
