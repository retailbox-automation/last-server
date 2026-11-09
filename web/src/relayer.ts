// Client for the studio relayer (POST /signup, POST /relay). "Unreachable" is distinct from
// "refused": only unreachable or overloaded relayers make the caller fall back to sending
// the transaction directly.

import { config } from './config.ts';

/**
 * The relayer could not serve the call. It may still have sent it (a timeout, or `504 relay_unknown`
 * with `txHash`), so the caller must read the chain before sending the same call another way.
 */
export class RelayerUnavailable extends Error {
  readonly txHash: string | undefined;
  constructor(message: string, txHash?: string) {
    super(message);
    this.name = 'RelayerUnavailable';
    this.txHash = txHash;
  }
}

export class RelayerRefused extends Error {
  readonly status: number;
  readonly code: string;
  readonly txHash: string | undefined;
  constructor(status: number, code: string, message: string, txHash?: string) {
    super(message);
    this.name = 'RelayerRefused';
    this.status = status;
    this.code = code;
    this.txHash = txHash;
  }
}

export interface RelayOk {
  txHash: string;
  /** Parsed JSON return value of the contract call, if it returned one. */
  result: unknown;
}

async function post(path: string, body: unknown, timeoutMs: number): Promise<{ status: number; json: any }> {
  if (config.relayerUrl === '') throw new RelayerUnavailable('no relayer configured');
  let res: Response;
  try {
    res = await fetch(config.relayerUrl + path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(timeoutMs),
    });
  } catch (e) {
    throw new RelayerUnavailable(`relayer unreachable: ${(e as Error).message}`);
  }
  let json: any = null;
  try {
    json = await res.json();
  } catch {
    // Non-JSON body (a proxy error page): treated as unavailable below if the status is 5xx.
  }
  return { status: res.status, json };
}

function toError(status: number, json: any): Error {
  const code: string = json?.error?.code ?? 'unknown';
  const message: string = json?.error?.message ?? `relayer answered ${status}`;
  // 5xx and 429 mean "this relayer cannot serve you now"; anything else is a decision about the request.
  if (status >= 500 || status === 429) return new RelayerUnavailable(`${status} ${code}: ${message}`, json?.txHash);
  return new RelayerRefused(status, code, message, json?.txHash);
}

export async function signup(name: string, publicKey: string): Promise<{ accountId: string; created: boolean }> {
  const { status, json } = await post('/signup', { name, publicKey }, 30_000);
  if (status !== 200) throw toError(status, json);
  return { accountId: json.accountId, created: json.created };
}

export async function relay(signedDelegateBase64: string): Promise<RelayOk> {
  const { status, json } = await post('/relay', { signedDelegate: signedDelegateBase64 }, 30_000);
  if (status !== 200) throw toError(status, json);
  let result: unknown = null;
  if (typeof json.resultBase64 === 'string' && json.resultBase64.length > 0) {
    try {
      result = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(json.resultBase64), (c) => c.charCodeAt(0))));
    } catch {
      result = null;
    }
  }
  return { txHash: json.txHash, result };
}
