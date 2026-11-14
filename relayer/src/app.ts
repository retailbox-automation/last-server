// HTTP layer: POST /signup, POST /relay, GET /health. Pure node:http, no framework.

import { createServer } from 'node:http';
import type { IncomingMessage, Server, ServerResponse } from 'node:http';
import { PublicKey } from 'near-api-js';
import { ChainError } from './chain.ts';
import type { Chain } from './chain.ts';
import type { PolicyConfig } from './config.ts';
import { RateLimiter } from './ratelimit.ts';
import { Rejection, checkDelegateWindow, playerNameProblem, validateSignedDelegate } from './validate.ts';

const MAX_BODY_BYTES = 32 * 1024;
/** Block height is read at most this often (NEAR makes about one block a second). */
const HEIGHT_CACHE_MS = 1_000;
/** Studio balance and the /health answer are reused for this long. */
const STATE_CACHE_MS = 5_000;

export interface AppOptions {
  policy: PolicyConfig;
  chain: Chain;
  /** One JSON line per event; defaults to stdout. */
  log?: (line: Record<string, unknown>) => void;
  now?: () => number;
  /** Pause used between the two signer-key checks; tests pass an instant one. */
  sleep?: (ms: number) => Promise<void>;
}

/** A brand-new player's key may not be visible to our RPC node yet: a failed key check is repeated once after this. */
export const SIGNER_RECHECK_MS = 1_000;

class HttpError extends Error {
  readonly status: number;
  readonly code: string;
  readonly retryAfterS: number | undefined;
  constructor(status: number, code: string, message: string, retryAfterS?: number) {
    super(message);
    this.name = 'HttpError';
    this.status = status;
    this.code = code;
    this.retryAfterS = retryAfterS;
  }
}

function clientIp(req: IncomingMessage, trustProxyHops: number): string {
  const socketIp = req.socket.remoteAddress ?? 'unknown';
  if (trustProxyHops <= 0) return socketIp;
  const forwarded = String(req.headers['x-forwarded-for'] ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  // Each trusted proxy appends the address it saw; the entry `hops` from the right is the client.
  return forwarded[forwarded.length - trustProxyHops] ?? socketIp;
}

async function readJson(req: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of req) {
    const buf = chunk as Buffer;
    size += buf.length;
    if (size > MAX_BODY_BYTES) throw new HttpError(413, 'body_too_large', 'request body is too large');
    chunks.push(buf);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } catch {
    throw new HttpError(400, 'bad_json', 'request body must be JSON');
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new HttpError(400, 'bad_json', 'request body must be a JSON object');
  }
  return parsed as Record<string, unknown>;
}

export function createApp(opts: AppOptions): Server {
  const { policy, chain } = opts;
  const now = opts.now ?? Date.now;
  const sleep = opts.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const log = opts.log ?? ((line) => console.log(JSON.stringify(line)));
  const l = policy.limits;
  const relayIp = new RateLimiter(l.relayIpMax, l.relayIpWindowS, now);
  const relaySender = new RateLimiter(l.relaySenderMax, l.relaySenderWindowS, now);
  const relayGlobal = new RateLimiter(l.relayGlobalMax, l.relayGlobalWindowS, now);
  const signupIp = new RateLimiter(l.signupIpMax, l.signupIpWindowS, now);
  const signupGlobal = new RateLimiter(l.signupGlobalMax, l.signupGlobalWindowS, now);
  const lookupIp = new RateLimiter(l.lookupIpMax, l.lookupIpWindowS, now);

  /** Memoizes an async read for `ttlMs`; concurrent callers share one request; failures are not kept. */
  function cached<T>(ttlMs: number, load: () => Promise<T>): () => Promise<T> {
    let value: { at: number; v: T } | null = null;
    let pending: Promise<T> | null = null;
    return () => {
      if (value !== null && now() - value.at < ttlMs) return Promise.resolve(value.v);
      pending ??= load()
        .then((v) => {
          value = { at: now(), v };
          return v;
        })
        .finally(() => {
          pending = null;
        });
      return pending;
    };
  }
  const blockHeight = cached(HEIGHT_CACHE_MS, () => chain.blockHeight());
  const studioBalance = cached(STATE_CACHE_MS, () => chain.studioBalance());
  /** Balance needed to fund one more player and keep the floor. */
  const signupReserve = policy.minStudioBalanceYocto + policy.signupFundingYocto;
  const healthState = cached(STATE_CACHE_MS, async () => {
    const [height, balance] = await Promise.all([blockHeight(), studioBalance()]);
    // A coarse flag only: the exact studio balance is nobody's business.
    return { blockHeight: Number(height), funds: balance >= signupReserve ? 'ok' : 'low' };
  });
  let proxyWarningLogged = false;

  function limit(limiter: RateLimiter, key: string, what: string): void {
    const r = limiter.hit(key);
    if (!r.ok) throw new HttpError(429, 'rate_limited', `too many ${what} requests`, r.retryAfterS);
  }

  function send(res: ServerResponse, status: number, body: unknown, headers: Record<string, string> = {}): void {
    const text = JSON.stringify(body);
    res.writeHead(status, {
      'content-type': 'application/json; charset=utf-8',
      'content-length': Buffer.byteLength(text),
      'access-control-allow-origin': policy.allowedOrigin,
      'cache-control': 'no-store',
      ...headers,
    });
    res.end(text);
  }

  async function signup(req: IncomingMessage, ip: string): Promise<{ status: number; body: unknown }> {
    // The name check costs an RPC call, so it has a budget of its own (not the /relay one).
    limit(lookupIp, `ip:${ip}`, 'lookup');
    const body = await readJson(req);
    const problem = playerNameProblem(body.name);
    if (problem !== null) throw new HttpError(400, 'bad_name', problem);
    const name = body.name as string;
    const keyText = body.publicKey;
    if (typeof keyText !== 'string' || !keyText.startsWith('ed25519:')) {
      throw new HttpError(400, 'bad_public_key', 'publicKey must look like "ed25519:<base58>"');
    }
    try {
      PublicKey.fromString(keyText);
    } catch {
      throw new HttpError(400, 'bad_public_key', 'publicKey is not a valid ed25519 key');
    }
    const accountId = `${name}.${policy.parentId}`;

    const status = await chain.accountStatus(accountId, keyText);
    if (status === 'has_key') return { status: 200, body: { accountId, created: false } };
    if (status === 'other_key') throw new HttpError(409, 'name_taken', 'this name is already taken');

    limit(signupIp, `ip:${ip}`, 'signup');
    limit(signupGlobal, 'global', 'signup');
    if ((await studioBalance()) < signupReserve) {
      throw new HttpError(503, 'studio_funds_low', 'the studio account is low on testnet NEAR; new players cannot be created right now');
    }
    try {
      const { txHash } = await chain.createPlayer(accountId, keyText);
      return { status: 200, body: { accountId, created: true, txHash } };
    } catch (e) {
      if (e instanceof ChainError && e.code === 'account_exists') {
        // Our own earlier attempt may have created it (e.g. a retry right after a timeout).
        if ((await chain.accountStatus(accountId, keyText)) === 'has_key') {
          return { status: 200, body: { accountId, created: false } };
        }
        throw new HttpError(409, 'name_taken', 'this name is already taken');
      }
      throw e;
    }
  }

  async function relay(req: IncomingMessage, ip: string): Promise<{ status: number; body: unknown; sender?: string }> {
    limit(relayIp, `ip:${ip}`, 'relay');
    const body = await readJson(req);
    const valid = validateSignedDelegate(body.signedDelegate, policy);
    // The signature only proves the delegate matches the key written inside it. Without this check anyone could
    // name a real player as sender, sign with a key of their own, and spend the sender and global budgets and the
    // studio's gas (the studio pays for failed calls too). The key must be an access key of the sender account.
    const signerKey = valid.signedDelegate.delegateAction.publicKey.toString();
    // The web client does not retry a 403 through the player's own account, so a key that is only a second late
    // must not be refused: look again once before saying no.
    if (!(await chain.isAccessKey(valid.senderId, signerKey))) {
      await sleep(SIGNER_RECHECK_MS);
      if (!(await chain.isAccessKey(valid.senderId, signerKey))) {
        throw new HttpError(403, 'bad_signer', 'the delegate is not signed by a key of the sender account');
      }
    }
    limit(relaySender, valid.senderId, 'relay');
    checkDelegateWindow(valid.maxBlockHeight, await blockHeight());
    // The studio pays the gas of every relayed call, failed ones included: stop at the balance floor,
    // and cap all relays together. A 503 or 429 makes the web client send the call from the player's own account.
    if ((await studioBalance()) < policy.minStudioBalanceYocto) {
      throw new HttpError(503, 'studio_funds_low', 'the studio account is low on testnet NEAR; send the call from your own account');
    }
    limit(relayGlobal, 'global', 'relay');
    const result = await chain.relay(valid.signedDelegate, policy.worldContractId);
    if (!result.ok) {
      return {
        status: 422,
        sender: valid.senderId,
        body: { error: { code: 'call_failed', message: result.failure ?? 'transaction failed' }, txHash: result.txHash },
      };
    }
    const payload: Record<string, unknown> = { ok: true, txHash: result.txHash, method: valid.method };
    if (result.resultBase64 !== undefined) payload.resultBase64 = result.resultBase64;
    return { status: 200, body: payload, sender: valid.senderId };
  }

  async function health(ip: string): Promise<{ status: number; body: unknown }> {
    limit(lookupIp, `ip:${ip}`, 'lookup');
    try {
      const h = await healthState();
      return {
        status: 200,
        body: { ok: true, parent: policy.parentId, world: policy.worldContractId, ...h },
      };
    } catch (e) {
      return { status: 503, body: { ok: false, error: { code: 'chain_error', message: (e as Error).message } } };
    }
  }

  const server = createServer((req, res) => {
    const started = now();
    const url = (req.url ?? '/').split('?')[0];
    const method = req.method ?? 'GET';
    const ip = clientIp(req, policy.trustProxyHops);
    if (!proxyWarningLogged && policy.trustProxyHops <= 0 && req.headers['x-forwarded-for'] !== undefined) {
      proxyWarningLogged = true;
      log({
        ev: 'warning',
        message: 'X-Forwarded-For is present but TRUST_PROXY_HOPS=0: every client behind the proxy shares one rate-limit key. Set TRUST_PROXY_HOPS to the number of proxies in front of the relayer.',
      });
    }
    const done = (status: number, extra: Record<string, unknown> = {}): void => {
      log({ ev: 'request', method, path: url, status, ip, ms: now() - started, ...extra });
    };

    void (async () => {
      try {
        if (method === 'OPTIONS') {
          res.writeHead(204, {
            'access-control-allow-origin': policy.allowedOrigin,
            'access-control-allow-methods': 'GET, POST, OPTIONS',
            'access-control-allow-headers': 'content-type',
            'access-control-max-age': '600',
          });
          res.end();
          return done(204);
        }
        if (method === 'GET' && url === '/health') {
          const r = await health(ip);
          send(res, r.status, r.body);
          return done(r.status);
        }
        if (method === 'POST' && url === '/signup') {
          const r = await signup(req, ip);
          send(res, r.status, r.body);
          return done(r.status);
        }
        if (method === 'POST' && url === '/relay') {
          const r = await relay(req, ip);
          send(res, r.status, r.body);
          return done(r.status, { sender: r.sender });
        }
        send(res, 404, { error: { code: 'not_found', message: 'unknown route' } });
        done(404);
      } catch (e) {
        let status = 500;
        let code = 'internal_error';
        let message = 'internal error';
        const headers: Record<string, string> = {};
        let txHash: string | undefined;
        if (e instanceof Rejection) {
          ({ status, code } = e);
          message = e.message;
        } else if (e instanceof HttpError) {
          ({ status, code } = e);
          message = e.message;
          if (e.retryAfterS !== undefined) headers['retry-after'] = String(e.retryAfterS);
        } else if (e instanceof ChainError) {
          // 504: sent, outcome unknown (look txHash up before retrying). 502: nothing executed.
          status = e.code === 'relay_unknown' ? 504 : 502;
          code = e.code;
          message = e.message.slice(0, 300);
          txHash = e.txHash;
        }
        const body = txHash === undefined ? { error: { code, message } } : { error: { code, message }, txHash };
        if (!res.headersSent) send(res, status, body, headers);
        done(status, { code });
      }
    })();
  });
  server.requestTimeout = 60_000;
  server.headersTimeout = 15_000;
  return server;
}
