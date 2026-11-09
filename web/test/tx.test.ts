// callWorld's retry policy: a relayer failure or a contract refusal is followed by a direct send
// only when the contract state shows the call did not happen. fetch is stubbed for both the relayer
// and the NEAR RPC, so no request leaves the process.
import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, it } from 'node:test';
import { KeyPair, baseEncode } from 'near-api-js';
import { config } from '../src/config.ts';
import { RelayerRefused } from '../src/relayer.ts';
import { CallFailed, callWorld } from '../src/tx.ts';
import type { AlreadyDone, CallResult } from '../src/tx.ts';

const realFetch = globalThis.fetch;
const RELAYER = 'http://relayer.test';
const session = { name: 'tom', accountId: 'tom.lastserver.testnet', secretKey: KeyPair.fromRandom('ed25519').toString() };
const HASH = baseEncode(new Uint8Array(32).fill(7));

let relayReply: { status: number; body: unknown };
/** Failure JSON a direct send gets back, or null for success. */
let directFailure: unknown = null;
let directSends = 0;
let relayCalls = 0;

function rpc(method: string): unknown {
  if (method === 'block') return { header: { height: 1000, hash: HASH } };
  if (method === 'query') return { block_height: 1000, block_hash: HASH, nonce: 1, permission: 'FullAccess' };
  if (method === 'send_tx') {
    directSends++;
    const status = directFailure === null ? { SuccessValue: Buffer.from('{"direct":true}').toString('base64') } : { Failure: directFailure };
    return { final_execution_status: 'EXECUTED_OPTIMISTIC', status, transaction: {}, transaction_outcome: { id: 'DIRECTTX', outcome: { status } }, receipts_outcome: [] };
  }
  throw new Error(`unexpected RPC ${method}`);
}

beforeEach(() => {
  config.relayerUrl = RELAYER;
  directFailure = null;
  directSends = 0;
  relayCalls = 0;
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    if (url.startsWith(RELAYER)) {
      relayCalls++;
      return new Response(JSON.stringify(relayReply.body), { status: relayReply.status });
    }
    const req = JSON.parse(String(init?.body));
    return new Response(JSON.stringify({ jsonrpc: '2.0', id: req.id, result: rpc(req.method) }), { status: 200 });
  }) as typeof fetch;
});
afterEach(() => {
  globalThis.fetch = realFetch;
  config.relayerUrl = '';
});

const done: CallResult = { txHash: 'SENT1', via: 'state', result: null };
/** An AlreadyDone that answers `answer` and records what it was asked. */
function probe(answer: CallResult | null): AlreadyDone & { calls: Array<string | undefined> } {
  const calls: Array<string | undefined> = [];
  const f = (async (txHash: string | undefined) => {
    calls.push(txHash);
    return answer;
  }) as AlreadyDone & { calls: Array<string | undefined> };
  f.calls = calls;
  return f;
}
const panic = (text: string) => ({ ActionError: { index: 0, kind: { FunctionCallError: { ExecutionError: `Smart contract panicked: ${text}` } } } });

describe('callWorld', () => {
  it('relayer 504 (sent, outcome unknown) and the call took effect: no direct resend', async () => {
    relayReply = { status: 504, body: { error: { code: 'relay_unknown', message: 'timeout' }, txHash: 'SENT1' } };
    const check = probe(done);
    const r = await callWorld(session, 'settle_dive', { dive_id: '1', moves: '' }, check);
    assert.equal(r, done);
    assert.deepEqual(check.calls, ['SENT1']);
    assert.equal(directSends, 0);
  });

  it('relayer timeout or network error and the state shows nothing happened: falls back to a direct send', async () => {
    relayReply = { status: 503, body: '<html>bad gateway</html>' };
    const check = probe(null);
    const r = await callWorld(session, 'start_dive', { loadout: [] }, check);
    assert.equal(r.via, 'direct');
    assert.deepEqual(r.result, { direct: true });
    assert.equal(check.calls.length, 1);
    assert.equal(directSends, 1);
  });

  it('a direct send refused because the relayed attempt landed meanwhile counts as done', async () => {
    relayReply = { status: 502, body: { error: { code: 'chain_error', message: 'x' } } };
    directFailure = panic('no active dive for this account');
    let n = 0;
    const check = (async () => (n++ === 0 ? null : done)) as AlreadyDone;
    const r = await callWorld(session, 'settle_dive', { dive_id: '1', moves: '' }, check);
    assert.equal(r, done);
    assert.equal(directSends, 1);
  });

  it('a contract refusal through the relayer: done if an earlier attempt went through, else CallFailed', async () => {
    relayReply = { status: 422, body: { error: { code: 'call_failed', message: '{"x":"Smart contract panicked: no active dive for this account"}' }, txHash: 'TXBAD' } };
    assert.equal(await callWorld(session, 'settle_dive', { dive_id: '1', moves: '' }, probe(done)), done);
    await assert.rejects(
      callWorld(session, 'settle_dive', { dive_id: '1', moves: '' }, probe(null)),
      (e: unknown) => e instanceof CallFailed && e.message === 'no active dive for this account' && e.txHash === 'TXBAD',
    );
    assert.equal(directSends, 0);
  });

  it('a policy refusal is final: no state read, no direct send', async () => {
    relayReply = { status: 403, body: { error: { code: 'delegate_expired', message: 'sign again' } } };
    const check = probe(done);
    await assert.rejects(callWorld(session, 'start_dive', { loadout: [] }, check), RelayerRefused);
    assert.equal(check.calls.length, 0);
    assert.equal(directSends, 0);
  });

  it('a relayed success needs no state read', async () => {
    relayReply = { status: 200, body: { ok: true, txHash: 'OK1', resultBase64: Buffer.from('{"a":1}').toString('base64') } };
    const check = probe(done);
    const r = await callWorld(session, 'start_dive', { loadout: [] }, check);
    assert.deepEqual(r, { txHash: 'OK1', via: 'relayer', result: { a: 1 } });
    assert.equal(check.calls.length, 0);
    assert.equal(relayCalls, 1);
  });
});
