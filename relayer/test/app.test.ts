import assert from 'node:assert/strict';
import type { AddressInfo } from 'node:net';
import { afterEach, describe, it } from 'node:test';
import type { Server } from 'node:http';
import { createApp } from '../src/app.ts';
import { ChainError } from '../src/chain.ts';
import type { AccountStatus, Chain, RelayResult } from '../src/chain.ts';
import { DEFAULT_LIMITS } from '../src/config.ts';
import type { Limits } from '../src/config.ts';
import { DEFAULT_MAX_BLOCK_HEIGHT, HEIGHT, NEAR, PARENT, WORLD, delegateBase64, policy, throwawayKey } from './helpers.ts';

class FakeChain implements Chain {
  accounts = new Map<string, string>(); // accountId -> public key
  created: string[] = [];
  relayed: string[] = [];
  relayResult: RelayResult = { txHash: 'TXHASH1', ok: true, resultBase64: Buffer.from('{"died":false}').toString('base64') };
  failNext: ChainError | null = null;
  healthy = true;
  height = HEIGHT;
  balance = 5n * NEAR;
  heightReads = 0;
  balanceReads = 0;
  /** Status reported after createPlayer failed with account_exists (simulates a lagging first read). */
  statusAfterExists: AccountStatus | null = null;

  async accountStatus(accountId: string, publicKey: string): Promise<AccountStatus> {
    if (this.statusAfterExists !== null && this.created.includes('!exists')) return this.statusAfterExists;
    const k = this.accounts.get(accountId);
    return k === undefined ? 'absent' : k === publicKey ? 'has_key' : 'other_key';
  }
  async createPlayer(accountId: string, publicKey: string): Promise<{ txHash: string }> {
    if (this.failNext) {
      if (this.failNext.code === 'account_exists') this.created.push('!exists');
      throw this.failNext;
    }
    this.accounts.set(accountId, publicKey);
    this.created.push(accountId);
    return { txHash: 'CREATE1' };
  }
  async relay(signed: Parameters<Chain['relay']>[0]): Promise<RelayResult> {
    if (this.failNext) throw this.failNext;
    this.relayed.push(signed.delegateAction.senderId);
    return this.relayResult;
  }
  async blockHeight() {
    this.heightReads++;
    if (!this.healthy) throw new ChainError('chain_error', 'rpc down');
    return this.height;
  }
  async studioBalance() {
    this.balanceReads++;
    if (!this.healthy) throw new ChainError('chain_error', 'rpc down');
    return this.balance;
  }
}

let open: Server | null = null;
afterEach(() => {
  open?.close();
  open = null;
});

async function start(over: { limits?: Partial<Limits>; trustProxyHops?: number; chain?: FakeChain } = {}) {
  const chain = over.chain ?? new FakeChain();
  let t = 1_000_000;
  const clock = { advance: (ms: number) => (t += ms) };
  const logs: Record<string, unknown>[] = [];
  const server = createApp({
    policy: policy({ limits: { ...DEFAULT_LIMITS, ...over.limits }, trustProxyHops: over.trustProxyHops ?? 0 }),
    chain,
    now: () => t,
    log: (line) => logs.push(line),
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  open = server;
  const base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  const post = async (path: string, body: unknown, headers: Record<string, string> = {}) => {
    const res = await fetch(base + path, {
      method: 'POST',
      headers: { 'content-type': 'application/json', ...headers },
      body: typeof body === 'string' ? body : JSON.stringify(body),
    });
    return { status: res.status, body: (await res.json()) as any, headers: res.headers };
  };
  return { chain, base, post, clock, logs };
}

describe('GET /health', () => {
  it('reports chain state', async () => {
    const { base } = await start();
    const res = await fetch(`${base}/health`);
    assert.equal(res.status, 200);
    const b: any = await res.json();
    assert.equal(b.ok, true);
    assert.equal(b.parent, PARENT);
    assert.equal(b.world, WORLD);
    assert.equal(b.blockHeight, Number(HEIGHT));
    assert.equal(b.funds, 'ok');
    assert.ok(!JSON.stringify(b).includes('secret'));
    // A coarse flag, not the exact studio balance.
    assert.ok(!('balanceNear' in b) && !JSON.stringify(b).includes(String(5n * NEAR)));
  });

  it('funds is "low" when the studio cannot fund one more player above the floor', async () => {
    const chain = new FakeChain();
    chain.balance = NEAR + NEAR / 10n; // floor 1 NEAR + 0.2 NEAR funding needed
    const { base } = await start({ chain });
    assert.equal(((await (await fetch(`${base}/health`)).json()) as any).funds, 'low');
  });

  it('is rate limited per IP and cached, so it cannot be used to burn the RPC budget', async () => {
    const { base, chain, clock } = await start({ limits: { lookupIpMax: 3 } });
    for (let i = 0; i < 3; i++) assert.equal((await fetch(`${base}/health`)).status, 200);
    assert.equal(chain.balanceReads, 1);
    const limited = await fetch(`${base}/health`);
    assert.equal(limited.status, 429);
    assert.ok(Number(limited.headers.get('retry-after')) >= 1);
    clock.advance(DEFAULT_LIMITS.lookupIpWindowS * 1000 + 1);
    assert.equal((await fetch(`${base}/health`)).status, 200);
    assert.equal(chain.balanceReads, 2);
  });

  it('503 when the chain is unreachable', async () => {
    const chain = new FakeChain();
    chain.healthy = false;
    const { base } = await start({ chain });
    const res = await fetch(`${base}/health`);
    assert.equal(res.status, 503);
    assert.equal(((await res.json()) as any).ok, false);
  });
});

describe('POST /signup', () => {
  const key = () => throwawayKey().getPublicKey().toString();

  it('creates <name>.<parent> and reports created: true', async () => {
    const { post, chain } = await start();
    const r = await post('/signup', { name: 'mara', publicKey: key() });
    assert.equal(r.status, 200);
    assert.deepEqual(r.body, { accountId: `mara.${PARENT}`, created: true, txHash: 'CREATE1' });
    assert.deepEqual(chain.created, [`mara.${PARENT}`]);
  });

  it('is idempotent for the same name and key', async () => {
    const { post, chain } = await start();
    const k = key();
    await post('/signup', { name: 'mara', publicKey: k });
    const again = await post('/signup', { name: 'mara', publicKey: k });
    assert.equal(again.status, 200);
    assert.equal(again.body.created, false);
    assert.equal(chain.created.length, 1);
  });

  it('409 when the name belongs to another key', async () => {
    const { post } = await start();
    await post('/signup', { name: 'mara', publicKey: key() });
    const r = await post('/signup', { name: 'mara', publicKey: key() });
    assert.equal(r.status, 409);
    assert.equal(r.body.error.code, 'name_taken');
  });

  it('409 when the chain says the account appeared in between', async () => {
    const chain = new FakeChain();
    chain.failNext = new ChainError('account_exists', 'account already exists');
    const { post } = await start({ chain });
    const r = await post('/signup', { name: 'late', publicKey: key() });
    assert.equal(r.status, 409);
  });

  it('400 for bad names', async () => {
    const { post, chain } = await start();
    for (const name of ['', 'ab', 'Mara', 'a.b', 'tom@example.com', 'world', 'x'.repeat(25), 7, null]) {
      const r = await post('/signup', { name, publicKey: key() });
      assert.equal(r.status, 400, String(name));
      assert.equal(r.body.error.code, 'bad_name');
    }
    assert.equal(chain.created.length, 0);
  });

  it('400 for bad keys', async () => {
    const { post } = await start();
    for (const publicKey of [undefined, 5, '', 'ed25519:', 'ed25519:notbase58!!', 'secp256k1:abc', 'abc']) {
      const r = await post('/signup', { name: 'mara', publicKey });
      assert.equal(r.status, 400, String(publicKey));
      assert.equal(r.body.error.code, 'bad_public_key');
    }
  });

  it('400 for bad JSON, 413 for a huge body', async () => {
    const { post } = await start();
    assert.equal((await post('/signup', '{not json')).status, 400);
    assert.equal((await post('/signup', '[1,2]')).status, 400);
    assert.equal((await post('/signup', JSON.stringify({ name: 'x'.repeat(40_000) }))).status, 413);
  });

  it('429 per IP once the signup budget is used, with Retry-After', async () => {
    const { post, chain, clock } = await start({ limits: { signupIpMax: 2 } });
    assert.equal((await post('/signup', { name: 'aaa', publicKey: key() })).status, 200);
    assert.equal((await post('/signup', { name: 'bbb', publicKey: key() })).status, 200);
    const r = await post('/signup', { name: 'ccc', publicKey: key() });
    assert.equal(r.status, 429);
    assert.equal(r.body.error.code, 'rate_limited');
    assert.ok(Number(r.headers.get('retry-after')) >= 1);
    assert.equal(chain.created.length, 2);
    clock.advance(DEFAULT_LIMITS.signupIpWindowS * 1000 + 1);
    assert.equal((await post('/signup', { name: 'ccc', publicKey: key() })).status, 200);
  });

  it('429 globally, whatever the IP', async () => {
    const { post } = await start({ limits: { signupGlobalMax: 1 }, trustProxyHops: 1 });
    assert.equal((await post('/signup', { name: 'aaa', publicKey: key() }, { 'x-forwarded-for': '1.1.1.1' })).status, 200);
    assert.equal((await post('/signup', { name: 'bbb', publicKey: key() }, { 'x-forwarded-for': '2.2.2.2' })).status, 429);
  });

  it('502 when the chain call fails', async () => {
    const chain = new FakeChain();
    chain.failNext = new ChainError('chain_error', 'rpc exploded');
    const { post } = await start({ chain });
    const r = await post('/signup', { name: 'mara', publicKey: key() });
    assert.equal(r.status, 502);
    assert.equal(r.body.error.code, 'chain_error');
  });

  it('503 studio_funds_low when funding a player would take the studio below the floor; relays still go', async () => {
    const chain = new FakeChain();
    chain.balance = NEAR + NEAR / 10n;
    const { post } = await start({ chain });
    const r = await post('/signup', { name: 'mara', publicKey: key() });
    assert.equal(r.status, 503);
    assert.equal(r.body.error.code, 'studio_funds_low');
    assert.equal(chain.created.length, 0);
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() })).status, 200);
  });

  it('does not spend the /relay budget of the same IP', async () => {
    const { post } = await start({ limits: { relayIpMax: 2 } });
    for (const name of ['aaa', 'bbb', 'ccc']) assert.equal((await post('/signup', { name, publicKey: key() })).status, 200);
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() })).status, 200);
  });

  it('name checks have their own per-IP budget', async () => {
    const { post } = await start({ limits: { lookupIpMax: 2 } });
    const k = key();
    assert.equal((await post('/signup', { name: 'mara', publicKey: k })).status, 200);
    assert.equal((await post('/signup', { name: 'mara', publicKey: k })).status, 200);
    const r = await post('/signup', { name: 'mara', publicKey: k });
    assert.equal(r.status, 429);
  });

  it('a create that collides with our own earlier create is reported as done, not taken', async () => {
    const chain = new FakeChain();
    chain.failNext = new ChainError('account_exists', 'account already exists');
    chain.statusAfterExists = 'has_key';
    const { post } = await start({ chain });
    const r = await post('/signup', { name: 'late', publicKey: key() });
    assert.equal(r.status, 200);
    assert.equal(r.body.created, false);
  });
});

describe('POST /relay: delegate lifetime (maxBlockHeight)', () => {
  it('403 delegate_expired for a delegate past (or about to pass) its maxBlockHeight; nothing is sent', async () => {
    const { post, chain } = await start();
    for (const maxBlockHeight of [HEIGHT - 10n, HEIGHT, HEIGHT + 4n]) {
      const r = await post('/relay', { signedDelegate: await delegateBase64({ maxBlockHeight }) });
      assert.equal(r.status, 403, String(maxBlockHeight));
      assert.equal(r.body.error.code, 'delegate_expired');
    }
    assert.equal(chain.relayed.length, 0);
  });

  it('403 delegate_ttl_too_long for a delegate meant to live far longer than the web signs', async () => {
    const { post, chain } = await start();
    for (const maxBlockHeight of [HEIGHT + 301n, 2n ** 63n]) {
      const r = await post('/relay', { signedDelegate: await delegateBase64({ maxBlockHeight }) });
      assert.equal(r.status, 403, String(maxBlockHeight));
      assert.equal(r.body.error.code, 'delegate_ttl_too_long');
    }
    assert.equal(chain.relayed.length, 0);
  });

  it('accepts the window edges (5 and 300 blocks ahead)', async () => {
    const { post } = await start();
    for (const maxBlockHeight of [HEIGHT + 5n, HEIGHT + 300n]) {
      assert.equal((await post('/relay', { signedDelegate: await delegateBase64({ maxBlockHeight }) })).status, 200);
    }
  });

  it('reads the block height at most once a second', async () => {
    const { post, chain, clock } = await start();
    for (let i = 0; i < 3; i++) await post('/relay', { signedDelegate: await delegateBase64() });
    assert.equal(chain.heightReads, 1);
    clock.advance(1_001);
    await post('/relay', { signedDelegate: await delegateBase64() });
    assert.equal(chain.heightReads, 2);
  });

  it('502 (not sent) when the height cannot be read', async () => {
    const chain = new FakeChain();
    chain.healthy = false;
    const { post } = await start({ chain });
    const r = await post('/relay', { signedDelegate: await delegateBase64({ maxBlockHeight: DEFAULT_MAX_BLOCK_HEIGHT }) });
    assert.equal(r.status, 502);
    assert.equal(chain.relayed.length, 0);
  });
});

describe('POST /relay', () => {
  it('503 studio_funds_low below the balance floor, without calling the chain', async () => {
    const chain = new FakeChain();
    chain.balance = NEAR - 1n;
    const { post } = await start({ chain });
    const r = await post('/relay', { signedDelegate: await delegateBase64() });
    assert.equal(r.status, 503);
    assert.equal(r.body.error.code, 'studio_funds_low');
    assert.equal(chain.relayed.length, 0);
  });

  it('429 for one signer over its daily budget, whatever the IP', async () => {
    const { post, chain } = await start({ limits: { relaySenderDailyMax: 2 }, trustProxyHops: 1 });
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() }, { 'x-forwarded-for': '1.1.1.1' })).status, 200);
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() }, { 'x-forwarded-for': '2.2.2.2' })).status, 200);
    const r = await post('/relay', { signedDelegate: await delegateBase64() }, { 'x-forwarded-for': '3.3.3.3' });
    assert.equal(r.status, 429);
    assert.equal(chain.relayed.length, 2);
  });

  it('relays at exactly the balance floor', async () => {
    const chain = new FakeChain();
    chain.balance = NEAR;
    const { post } = await start({ chain });
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() })).status, 200);
  });

  it('429 for all relays together once the global budget is spent, whatever the IP; resets after the window', async () => {
    const { post, clock, chain } = await start({ limits: { relayGlobalMax: 2 }, trustProxyHops: 1 });
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() }, { 'x-forwarded-for': '1.1.1.1' })).status, 200);
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() }, { 'x-forwarded-for': '2.2.2.2' })).status, 200);
    const r = await post('/relay', { signedDelegate: await delegateBase64() }, { 'x-forwarded-for': '3.3.3.3' });
    assert.equal(r.status, 429);
    assert.equal(r.body.error.code, 'rate_limited');
    assert.equal(chain.relayed.length, 2);
    clock.advance(DEFAULT_LIMITS.relayGlobalWindowS * 1000 + 1);
    assert.equal((await post('/relay', { signedDelegate: await delegateBase64() }, { 'x-forwarded-for': '3.3.3.3' })).status, 200);
  });

  it('forwards a valid start_dive and returns the tx hash and the call result', async () => {
    const { post, chain } = await start();
    const r = await post('/relay', { signedDelegate: await delegateBase64({ sender: `tom.${PARENT}` }) });
    assert.equal(r.status, 200);
    assert.equal(r.body.ok, true);
    assert.equal(r.body.txHash, 'TXHASH1');
    assert.equal(r.body.method, 'start_dive');
    assert.equal(Buffer.from(r.body.resultBase64, 'base64').toString(), '{"died":false}');
    assert.deepEqual(chain.relayed, [`tom.${PARENT}`]);
  });

  it('403 for every forbidden shape, and nothing reaches the chain', async () => {
    const { post, chain } = await start();
    const cases: Array<[string, Parameters<typeof delegateBase64>[0], string]> = [
      ['wrong receiver', { receiver: 'evil.testnet' }, 'wrong_receiver'],
      ['wrong method', { method: 'mt_transfer' }, 'forbidden_method'],
      ['deposit', { deposit: 1n }, 'deposit_not_allowed'],
      ['gas', { gas: 100_000_000_000_001n }, 'gas_too_high'],
      ['sender', { sender: 'bob.testnet' }, 'wrong_sender'],
    ];
    for (const [label, spec, code] of cases) {
      const r = await post('/relay', { signedDelegate: await delegateBase64(spec) });
      assert.equal(r.status, 403, label);
      assert.equal(r.body.error.code, code, label);
    }
    assert.equal(chain.relayed.length, 0);
  });

  it('400 for garbage and missing field', async () => {
    const { post } = await start();
    assert.equal((await post('/relay', {})).status, 400);
    assert.equal((await post('/relay', { signedDelegate: 'AAAA' })).status, 400);
    assert.equal((await post('/relay', 'nope')).status, 400);
  });

  it('429 per sender, other senders unaffected', async () => {
    const { post, chain, clock } = await start({ limits: { relaySenderMax: 2 } });
    const send = async (sender: string) => post('/relay', { signedDelegate: await delegateBase64({ sender }) });
    assert.equal((await send(`tom.${PARENT}`)).status, 200);
    assert.equal((await send(`tom.${PARENT}`)).status, 200);
    const limited = await send(`tom.${PARENT}`);
    assert.equal(limited.status, 429);
    assert.ok(Number(limited.headers.get('retry-after')) >= 1);
    assert.equal((await send(`mara.${PARENT}`)).status, 200);
    assert.equal(chain.relayed.length, 3);
    clock.advance(DEFAULT_LIMITS.relaySenderWindowS * 1000 + 1);
    assert.equal((await send(`tom.${PARENT}`)).status, 200);
  });

  it('429 per IP, even for invalid requests (rejects cost budget too)', async () => {
    const { post } = await start({ limits: { relayIpMax: 3 } });
    for (let i = 0; i < 3; i++) assert.equal((await post('/relay', {})).status, 400);
    const r = await post('/relay', { signedDelegate: await delegateBase64() });
    assert.equal(r.status, 429);
  });

  it('IP limit keys on X-Forwarded-For only when proxies are trusted', async () => {
    const direct = await start({ limits: { relayIpMax: 1 } });
    assert.equal((await direct.post('/relay', {}, { 'x-forwarded-for': '9.9.9.1' })).status, 400);
    // Header ignored without TRUST_PROXY_HOPS: same socket address, second request is limited.
    assert.equal((await direct.post('/relay', {}, { 'x-forwarded-for': '9.9.9.2' })).status, 429);
    open?.close();

    const proxied = await start({ limits: { relayIpMax: 1 }, trustProxyHops: 1 });
    assert.equal((await proxied.post('/relay', {}, { 'x-forwarded-for': 'spoofed, 9.9.9.1' })).status, 400);
    assert.equal((await proxied.post('/relay', {}, { 'x-forwarded-for': 'spoofed2, 9.9.9.2' })).status, 400);
    assert.equal((await proxied.post('/relay', {}, { 'x-forwarded-for': 'spoofed3, 9.9.9.1' })).status, 429);
  });

  it('422 with the failure text when the contract call fails', async () => {
    const chain = new FakeChain();
    chain.relayResult = { txHash: 'TXBAD', ok: false, failure: '{"ActionError":"Smart contract panicked: no active dive"}' };
    const { post } = await start({ chain });
    const r = await post('/relay', { signedDelegate: await delegateBase64({ method: 'settle_dive' }) });
    assert.equal(r.status, 422);
    assert.equal(r.body.error.code, 'call_failed');
    assert.match(r.body.error.message, /no active dive/);
    assert.equal(r.body.txHash, 'TXBAD');
  });

  it('504 relay_unknown with the tx hash when the transaction was sent but its outcome is unknown', async () => {
    const chain = new FakeChain();
    chain.failNext = new ChainError('relay_unknown', 'relay outcome unknown: timeout', 'TXSENT');
    const { post } = await start({ chain });
    const r = await post('/relay', { signedDelegate: await delegateBase64() });
    assert.equal(r.status, 504);
    assert.equal(r.body.error.code, 'relay_unknown');
    assert.equal(r.body.txHash, 'TXSENT');
  });

  it('502 when the RPC fails', async () => {
    const chain = new FakeChain();
    chain.failNext = new ChainError('chain_error', 'relay failed: 429 TooManyRequests');
    const { post } = await start({ chain });
    const r = await post('/relay', { signedDelegate: await delegateBase64() });
    assert.equal(r.status, 502);
    assert.equal(r.body.error.code, 'chain_error');
  });
});

describe('plumbing', () => {
  it('answers CORS preflight and sets the allowed origin', async () => {
    const { base } = await start();
    const pre = await fetch(`${base}/relay`, { method: 'OPTIONS' });
    assert.equal(pre.status, 204);
    assert.equal(pre.headers.get('access-control-allow-origin'), '*');
    assert.match(pre.headers.get('access-control-allow-methods') ?? '', /POST/);
  });

  it('404 for unknown routes and wrong methods', async () => {
    const { base } = await start();
    assert.equal((await fetch(`${base}/nope`)).status, 404);
    assert.equal((await fetch(`${base}/relay`)).status, 404);
  });

  it('warns once when X-Forwarded-For arrives but no proxy is trusted', async () => {
    const { post, logs } = await start();
    await post('/relay', {}, { 'x-forwarded-for': '9.9.9.1' });
    await post('/relay', {}, { 'x-forwarded-for': '9.9.9.2' });
    const warnings = logs.filter((l) => l.ev === 'warning');
    assert.equal(warnings.length, 1);
    assert.match(String(warnings[0]!.message), /TRUST_PROXY_HOPS/);
  });

  it('no proxy warning without the header, or when proxies are trusted', async () => {
    const a = await start();
    await a.post('/relay', {});
    assert.equal(a.logs.filter((l) => l.ev === 'warning').length, 0);
    open?.close();
    const b = await start({ trustProxyHops: 1 });
    await b.post('/relay', {}, { 'x-forwarded-for': '9.9.9.1' });
    assert.equal(b.logs.filter((l) => l.ev === 'warning').length, 0);
  });

  it('request log lines never contain the delegate or keys', async () => {
    const { post, logs } = await start();
    const b64 = await delegateBase64({ sender: `tom.${PARENT}` });
    await post('/relay', { signedDelegate: b64 });
    const text = JSON.stringify(logs);
    assert.ok(text.includes('"status":200'));
    assert.ok(!text.includes(b64));
    assert.ok(!/secret|ed25519:/i.test(text));
  });
});
