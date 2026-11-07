// Local stand-in for NEAR testnet + world contract + studio account, for developing the web client
// without a deployed contract: a JSON-RPC server (views, access keys, send_tx) and the real relayer
// code running on a fake chain. It runs the rules through the SAME wasm the browser uses, so it
// cannot catch a rules bug; it exists to exercise the UI, the signed-delegate path and the fallback.
//
//   node dev/mock-world.mjs            # RPC on :3030, relayer on :8787
//   VITE_RPC_URL=http://localhost:3030 VITE_RELAYER_URL=http://localhost:8787 npm run dev   (in web/)
// Needs web/src/wasm (npm run wasm in web/). Not a test of the contract. Nothing here touches a network.

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { decodeSignedTransaction, baseEncode, PublicKey } from 'near-api-js';
import { createApp } from '../src/app.ts';
import { DEFAULT_LIMITS } from '../src/config.ts';
import { initSync, item_names, run_dive, core_hash } from '../../web/src/wasm/ls_wasm.js';

initSync({ module: readFileSync(new URL('../../web/src/wasm/ls_wasm_bg.wasm', import.meta.url)) });

const PARENT = 'lastserver.testnet';
const WORLD = 'world.lastserver.testnet';
const RPC_PORT = Number(process.env.MOCK_RPC_PORT ?? 3030);
const RELAYER_PORT = Number(process.env.MOCK_RELAYER_PORT ?? 8787);

const names = new Map(JSON.parse(item_names())); // id -> token
const ids = new Map([...names].map(([id, n]) => [n, id])); // token -> id

// ---- world state ------------------------------------------------------------------------------
const day = () => Math.floor(Date.now() / 86_400_000);
const seedOf = (d) => createHash('sha256').update(`day:${d}:mock-salt`).digest().readBigUInt64BE(0);
const balances = new Map(); // account -> Map(token -> n)
const locked = new Map(); // account -> Map(token -> n)
const active = new Map(); // account -> { ticket, loadout: [[id, n]] }
const stats = new Map(); // account -> { dives, deaths, deepest }
let graves = [
  { id: 1, owner_id: `mara.${PARENT}`, depth: 3, x: 10, y: 10, items: [[ids.get('gold'), 12], [ids.get('blade'), 1]] },
  { id: 2, owner_id: `tom.${PARENT}`, depth: 2, x: 20, y: 12, items: [[ids.get('plate'), 1]] },
];
let nextGrave = 3;
let nextDive = 1;

const bal = (a) => balances.get(a) ?? balances.set(a, new Map()).get(a);
const lock = (a) => locked.get(a) ?? locked.set(a, new Map()).get(a);
const stat = (a) => stats.get(a) ?? stats.set(a, { dives: 0, deaths: 0, deepest: 0 }).get(a);
const add = (m, k, n) => m.set(k, (m.get(k) ?? 0) + n);
const pairs = (m) => [...m].filter(([, n]) => n > 0);

class Panic extends Error {}

function entryOf(account) {
  const a = active.get(account);
  return { start_depth: a?.ticket.start_depth ?? 1, inventory: a?.loadout ?? [], graves: [] };
}

function view(method, args) {
  switch (method) {
    case 'current_day': return day();
    case 'day_seed': return String(seedOf(args.day));
    case 'active_dive': return active.get(args.account_id)?.ticket ?? null;
    case 'dive_entry': return entryOf(args.account_id);
    case 'graves': return graves.filter((g) => g.depth === args.depth);
    case 'graves_summary': {
      const c = new Map();
      for (const g of graves) c.set(g.depth, (c.get(g.depth) ?? 0) + 1);
      return [...c].sort((x, y) => x[0] - y[0]);
    }
    case 'player': {
      const a = args.account_id;
      return {
        stash: pairs(bal(a)).map(([t, n]) => [t, String(n)]),
        locked: pairs(lock(a)).map(([t, n]) => [t, String(n)]),
        ...stat(a),
      };
    }
    case 'core_version': return `mock ls-core core-hash ${core_hash()}`;
    default: throw new Panic(`no such view: ${method}`);
  }
}

function call(sender, method, args) {
  if (method === 'start_dive') {
    if (active.has(sender)) throw new Panic('a dive is already active');
    const loadout = [];
    for (const [token, amount] of args.loadout ?? []) {
      const n = Number(amount);
      if (!ids.has(token) || !(n > 0)) throw new Panic(`bad loadout item ${token}`);
      if ((bal(sender).get(token) ?? 0) - (lock(sender).get(token) ?? 0) < n) throw new Panic(`not enough ${token}`);
      add(lock(sender), token, n);
      loadout.push([ids.get(token), n]);
    }
    const ticket = { dive_id: String(nextDive++), day: day(), seed: String(seedOf(day())), start_depth: 1 };
    active.set(sender, { ticket, loadout });
    return ticket;
  }
  if (method === 'settle_dive') {
    const a = active.get(sender);
    if (!a || a.ticket.dive_id !== String(args.dive_id)) throw new Panic('no active dive');
    const moves = Buffer.from(args.moves, 'base64');
    if (moves.length > 4000) throw new Panic('move log too long');
    const entry = { start_depth: a.ticket.start_depth, inventory: a.loadout, graves: graves.map(({ id, depth, x, y, items }) => ({ id, depth, x, y, items })) };
    const o = JSON.parse(run_dive(BigInt(a.ticket.seed), JSON.stringify(entry), moves));
    // Apply. Unlock the loadout, burn what the rules burned, mint finds, move graves.
    for (const [t, n] of pairs(lock(sender))) add(lock(sender), t, -n);
    for (const [id, n] of o.burned) add(bal(sender), names.get(id), -n);
    for (const [id, n] of o.picked) add(bal(sender), names.get(id), n);
    for (const gid of o.looted_graves) {
      const g = graves.find((x) => x.id === gid);
      if (!g) continue;
      for (const [id, n] of g.items) add(bal(sender), names.get(id), n);
      graves = graves.filter((x) => x.id !== gid);
    }
    if (o.died) {
      for (const [id, n] of o.dropped_into_grave) add(bal(sender), names.get(id), -n);
      graves.push({ id: nextGrave++, owner_id: sender, depth: o.final_depth, x: o.death_pos?.[0] ?? 1, y: o.death_pos?.[1] ?? 1, items: o.dropped_into_grave });
    }
    const s = stat(sender);
    s.dives += 1;
    s.deaths += o.died ? 1 : 0;
    s.deepest = Math.max(s.deepest, o.final_depth);
    active.delete(sender);
    return o;
  }
  throw new Panic(`method ${method} is not callable in the mock`);
}

// ---- fake chain for the real relayer ----------------------------------------------------------
const accounts = new Map(); // accountId -> public key string
let txCounter = 1;
const txId = () => `MOCKTX${String(txCounter++).padStart(6, '0')}`;

function runAction(sender, methodName, argsBytes) {
  const args = argsBytes.length ? JSON.parse(Buffer.from(argsBytes).toString('utf8')) : {};
  try {
    const result = call(sender, methodName, args);
    return { ok: true, resultBase64: Buffer.from(JSON.stringify(result)).toString('base64') };
  } catch (e) {
    if (e instanceof Panic) return { ok: false, failure: `{"ActionError":{"kind":{"FunctionCallError":{"ExecutionError":"Smart contract panicked: ${e.message}"}}}}` };
    throw e;
  }
}

const chain = {
  async accountStatus(id, pk) {
    const k = accounts.get(id);
    return k === undefined ? 'absent' : k === pk ? 'has_key' : 'other_key';
  },
  async createPlayer(id, pk) {
    accounts.set(id, pk);
    return { txHash: txId() };
  },
  async relay(signed) {
    const da = signed.delegateAction;
    const fc = da.actions[0].functionCall;
    const r = runAction(da.senderId, fc.methodName, fc.args);
    return { txHash: txId(), ...r };
  },
  async health() {
    return { blockHeight: 1, balanceNear: 'mock' };
  },
};

const policy = { parentId: PARENT, worldContractId: WORLD, allowedOrigin: '*', trustProxyHops: 0, limits: { ...DEFAULT_LIMITS, signupIpMax: 1000, relayIpMax: 100000, relaySenderMax: 100000 } };
createApp({ policy, chain }).listen(RELAYER_PORT, '127.0.0.1', () => console.log(`mock relayer on http://127.0.0.1:${RELAYER_PORT}`));

// ---- JSON-RPC ---------------------------------------------------------------------------------
const blockHash = baseEncode(createHash('sha256').update('mock-block').digest());
const header = () => ({ height: 1_000_000 + Math.floor(Date.now() / 1000), hash: blockHash });

function rpc(method, params) {
  if (method === 'block') return { header: header(), chunks: [], author: 'mock' };
  if (method === 'gas_price') return { gas_price: '100000000' };
  if (method === 'query') {
    const base = { block_height: header().height, block_hash: blockHash };
    switch (params.request_type) {
      case 'call_function': {
        const args = params.args_base64 ? JSON.parse(Buffer.from(params.args_base64, 'base64').toString() || '{}') : {};
        return { ...base, logs: [], result: [...Buffer.from(JSON.stringify(view(params.method_name, args)))] };
      }
      case 'view_access_key': {
        // Like the real RPC: query failures come back as result.error strings.
        const id = params.account_id;
        if (id !== PARENT) {
          if (!accounts.has(id)) return { ...base, logs: [], error: `account ${id} does not exist while viewing` };
          if (accounts.get(id) !== params.public_key) return { ...base, logs: [], error: `access key ${params.public_key} does not exist while viewing` };
        }
        return { ...base, nonce: 1, permission: 'FullAccess' };
      }
      case 'view_account': return { ...base, amount: '1000000000000000000000000', locked: '0', code_hash: '11111111111111111111111111111111', storage_usage: 100, storage_paid_at: 0, global_contract_hash: null, global_contract_account_id: null };
      default: throw Object.assign(new Error('unsupported query'), { rpc: true });
    }
  }
  if (method === 'send_tx' || method === 'broadcast_tx_commit') {
    const signed = decodeSignedTransaction(Buffer.from(params.signed_tx_base64 ?? params[0], 'base64'));
    const t = signed.transaction;
    const first = t.actions[0];
    let r;
    if (first?.createAccount) {
      const key = PublicKey.from(t.actions.find((a) => a.addKey).addKey.publicKey).toString();
      if (accounts.has(t.receiverId)) r = { ok: false, failure: JSON.stringify({ ActionError: { index: 0, kind: { AccountAlreadyExists: { account_id: t.receiverId } } } }) };
      else { accounts.set(t.receiverId, key); r = { ok: true, resultBase64: '' }; }
    } else if (first?.signedDelegate) {
      const da = first.signedDelegate.delegateAction;
      const fc = da.actions[0].functionCall;
      r = runAction(da.senderId, fc.methodName, fc.args);
    } else if (first?.functionCall) {
      r = runAction(t.signerId, first.functionCall.methodName, first.functionCall.args);
    } else throw new Error('mock: unsupported action');
    const status = r.ok ? { SuccessValue: r.resultBase64 } : { Failure: JSON.parse(r.failure) };
    return { final_execution_status: 'EXECUTED_OPTIMISTIC', status, transaction: {}, transaction_outcome: { id: txId(), outcome: { status } }, receipts_outcome: [] };
  }
  throw new Error(`mock: unsupported RPC method ${method}`);
}

createServer((req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    const cors = { 'access-control-allow-origin': '*', 'access-control-allow-headers': 'content-type', 'content-type': 'application/json' };
    if (req.method === 'OPTIONS') { res.writeHead(204, cors); return res.end(); }
    let body = {};
    try { body = JSON.parse(Buffer.concat(chunks).toString() || '{}'); } catch { /* ignore */ }
    let out;
    try {
      out = { jsonrpc: '2.0', id: body.id, result: rpc(body.method, Array.isArray(body.params) ? body.params : body.params ?? {}) };
    } catch (e) {
      out = { jsonrpc: '2.0', id: body.id, error: { code: -32000, message: 'Server error', data: String(e.message), name: 'HANDLER_ERROR', cause: { name: 'UNKNOWN', info: { error_message: String(e.message) } } } };
    }
    res.writeHead(200, cors);
    res.end(JSON.stringify(out, (_, v) => (typeof v === 'bigint' ? String(v) : v)));
  });
}).listen(RPC_PORT, '127.0.0.1', () => console.log(`mock RPC on http://127.0.0.1:${RPC_PORT} (world ${WORLD})`));
