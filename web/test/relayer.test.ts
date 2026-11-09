import assert from 'node:assert/strict';
import { afterEach, beforeEach, describe, it } from 'node:test';
import { config, nameProblem, readConfig } from '../src/config.ts';
import { RelayerRefused, RelayerUnavailable, relay, signup } from '../src/relayer.ts';
import { failureMessage } from '../src/tx.ts';

const realFetch = globalThis.fetch;
function mockFetch(status: number, body: unknown): void {
  globalThis.fetch = (async () =>
    new Response(typeof body === 'string' ? body : JSON.stringify(body), { status })) as typeof fetch;
}

beforeEach(() => {
  config.relayerUrl = 'http://relayer.test';
});
afterEach(() => {
  globalThis.fetch = realFetch;
  config.relayerUrl = '';
});

describe('relayer client', () => {
  it('relay: decodes the contract result', async () => {
    const result = Buffer.from('{"dive_id":"3","seed":"99"}').toString('base64');
    mockFetch(200, { ok: true, txHash: 'H1', resultBase64: result });
    const r = await relay('AAAA');
    assert.equal(r.txHash, 'H1');
    assert.deepEqual(r.result, { dive_id: '3', seed: '99' });
  });

  it('relay: a policy refusal is RelayerRefused (no direct fallback)', async () => {
    mockFetch(403, { error: { code: 'forbidden_method', message: 'only start_dive and settle_dive are allowed' } });
    await assert.rejects(relay('AAAA'), (e: unknown) => e instanceof RelayerRefused && e.status === 403 && e.code === 'forbidden_method');
  });

  it('relay: a failed contract call carries the tx hash', async () => {
    mockFetch(422, { error: { code: 'call_failed', message: 'panicked' }, txHash: 'HBAD' });
    await assert.rejects(relay('AAAA'), (e: unknown) => e instanceof RelayerRefused && e.txHash === 'HBAD');
  });

  it('relay: 429, 502, 503 and HTML error pages mean unavailable (fallback allowed)', async () => {
    for (const [status, body] of [[429, { error: { code: 'rate_limited', message: 'x' } }], [502, { error: { code: 'chain_error', message: 'x' } }], [503, '<html>bad gateway</html>']] as const) {
      mockFetch(status, body);
      await assert.rejects(relay('AAAA'), RelayerUnavailable, String(status));
    }
  });

  it('relay: a network error means unavailable', async () => {
    globalThis.fetch = (async () => {
      throw new TypeError('fetch failed');
    }) as typeof fetch;
    await assert.rejects(relay('AAAA'), RelayerUnavailable);
  });

  it('no relayer configured means unavailable without any request', async () => {
    config.relayerUrl = '';
    let called = false;
    globalThis.fetch = (async () => {
      called = true;
      return new Response('{}');
    }) as typeof fetch;
    await assert.rejects(relay('AAAA'), RelayerUnavailable);
    assert.equal(called, false);
  });

  it('signup: ok, taken, and offline', async () => {
    mockFetch(200, { accountId: 'tom.lastserver.testnet', created: true });
    assert.deepEqual(await signup('tom', 'ed25519:x'), { accountId: 'tom.lastserver.testnet', created: true });
    mockFetch(409, { error: { code: 'name_taken', message: 'taken' } });
    await assert.rejects(signup('tom', 'ed25519:x'), (e: unknown) => e instanceof RelayerRefused && e.code === 'name_taken');
    mockFetch(503, '');
    await assert.rejects(signup('tom', 'ed25519:x'), RelayerUnavailable);
  });
});

describe('config and names', () => {
  it('defaults', () => {
    const c = readConfig({});
    assert.equal(c.rpcUrl, 'https://test.rpc.fastnear.com');
    assert.equal(c.worldContractId, 'world.lastserver.testnet');
    assert.equal(c.relayerUrl, '');
    assert.equal(readConfig({ DEV: true }).relayerUrl, 'http://localhost:8787');
  });

  it('overrides and trailing slash', () => {
    const c = readConfig({ VITE_RPC_URL: 'http://rpc', VITE_WORLD_CONTRACT_ID: 'w.testnet', VITE_RELAYER_URL: 'https://r.example/' });
    assert.deepEqual([c.rpcUrl, c.worldContractId, c.relayerUrl], ['http://rpc', 'w.testnet', 'https://r.example']);
  });

  it('name rules match the relayer', () => {
    for (const ok of ['tom', 'mara-7', '007']) assert.equal(nameProblem(ok), null, ok);
    for (const bad of ['', 'ab', 'Tom', 'a b', 'a--b', 'world', 'tom@example.com', 'x'.repeat(25)]) assert.notEqual(nameProblem(bad), null, bad);
  });
});

describe('failureMessage', () => {
  it('extracts the contract panic text', () => {
    const f = JSON.stringify({ ActionError: { index: 0, kind: { FunctionCallError: { ExecutionError: 'Smart contract panicked: no active dive' } } } });
    assert.equal(failureMessage(f), 'no active dive');
  });
  it('falls back to the raw text', () => {
    assert.equal(failureMessage('{"X":1}'), '{"X":1}');
  });
});
