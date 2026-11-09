// Reads from the world contract. The whole RPC surface of the page is this file plus tx.ts,
// so pointing the game at another endpoint or contract is a configuration change.

import { JsonRpcProvider, PublicKey } from 'near-api-js';
import { AccessKeyDoesNotExistError, AccountDoesNotExistError } from 'near-api-js/rpc-errors';
import { config } from './config.ts';

export const provider = new JsonRpcProvider({ url: config.rpcUrl });

/** Calls a view method of the world contract and returns its parsed JSON result. */
export async function view<T>(method: string, args: Record<string, unknown> = {}): Promise<T> {
  const r = await provider.callFunction<any>({
    contractId: config.worldContractId,
    method,
    args,
    blockQuery: { finality: 'optimistic' },
  });
  return r as T;
}

export function explorerTxUrl(txHash: string): string {
  return config.explorerTxBase + txHash;
}

/** The RPC does not (yet) know this account or key. */
export function isMissingKey(e: unknown): boolean {
  return e instanceof AccessKeyDoesNotExistError || e instanceof AccountDoesNotExistError;
}

export interface KeyWaitOptions {
  provider?: Pick<JsonRpcProvider, 'viewAccessKey'>;
  timeoutMs?: number;
  intervalMs?: number;
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
}

/**
 * Waits until the RPC sees a just-added access key, first at optimistic and then at final finality:
 * an account created a moment ago can still be unknown to the RPC node that answers the next
 * request, and signing needs the key's nonce. Polls every `intervalMs` for up to `timeoutMs`.
 */
export async function waitForAccessKey(accountId: string, publicKey: string, opts: KeyWaitOptions = {}): Promise<void> {
  const rpc = opts.provider ?? provider;
  const timeoutMs = opts.timeoutMs ?? 15_000;
  const intervalMs = opts.intervalMs ?? 750;
  const sleep = opts.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const now = opts.now ?? Date.now;
  const deadline = now() + timeoutMs;
  const pk = PublicKey.fromString(publicKey);
  for (const finality of ['optimistic', 'final'] as const) {
    for (;;) {
      try {
        await rpc.viewAccessKey({ accountId, publicKey: pk, finalityQuery: { finality } });
        break;
      } catch (e) {
        // Not there yet, or a passing RPC error: both are worth another look until the deadline.
        if (now() >= deadline) {
          throw new Error(
            `Your account ${accountId} was created, but the NEAR RPC does not show its key yet${isMissingKey(e) ? '' : ` (${(e as Error).message})`}. Wait a minute and press Play again.`,
          );
        }
        await sleep(intervalMs);
      }
    }
  }
}

export async function accountExists(accountId: string): Promise<boolean> {
  try {
    await provider.viewAccount({ accountId, blockQuery: { finality: 'optimistic' } });
    return true;
  } catch (e) {
    // instanceof, not the error's name: a minified build renames classes.
    if (e instanceof AccountDoesNotExistError) return false;
    throw e;
  }
}
