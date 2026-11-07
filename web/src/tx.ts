// Sends a call to the world contract as the player: through the relayer as a NEP-366 signed
// delegate (the player pays nothing and sees no prompt), or, if the relayer is down or overloaded,
// directly with the player's own key and 100 Tgas.

import {
  Account,
  KeyPair,
  KeyPairSigner,
  actions,
  encodeSignedDelegate,
  getTransactionLastResult,
} from 'near-api-js';
import { toBase64 } from './bytes.ts';
import { provider } from './chain.ts';
import { config } from './config.ts';
import type { Session } from './keys.ts';
import { RelayerRefused, RelayerUnavailable, relay } from './relayer.ts';

export const GAS_100_TGAS = 100_000_000_000_000n;
/** A signed delegate expires after this many blocks (about 2 minutes), so a stuck one cannot be replayed later. */
const DELEGATE_TTL_BLOCKS = 100;

export interface CallResult {
  txHash: string;
  via: 'relayer' | 'direct';
  /** Parsed JSON return value of the contract method. */
  result: unknown;
}

/** The contract rejected or failed the call (panic message in `message`). */
export class CallFailed extends Error {
  readonly txHash: string | undefined;
  constructor(message: string, txHash?: string) {
    super(message);
    this.name = 'CallFailed';
    this.txHash = txHash;
  }
}

function playerAccount(session: Session): Account {
  return new Account(session.accountId, provider, new KeyPairSigner(KeyPair.fromString(session.secretKey as `ed25519:${string}`)));
}

/** Text of the first failure inside a transaction outcome, or null. */
function failureOf(outcome: any): string | null {
  const find = (status: any): string | null =>
    status && typeof status === 'object' && 'Failure' in status ? JSON.stringify(status.Failure) : null;
  const top = find(outcome?.status);
  if (top) return top;
  for (const r of outcome?.receipts_outcome ?? []) {
    const f = find(r?.outcome?.status);
    if (f) return f;
  }
  return null;
}

/** Pulls the human part out of a NEAR failure JSON, e.g. the contract's panic text. */
export function failureMessage(failureJson: string): string {
  const m = /Smart contract panicked: ([^"\\]*)/.exec(failureJson);
  return m?.[1] ?? failureJson.slice(0, 300);
}

export async function callWorld(session: Session, method: string, args: Record<string, unknown>): Promise<CallResult> {
  const account = playerAccount(session);

  if (config.relayerUrl !== '') {
    try {
      const { signedDelegate } = await account.createSignedMetaTransaction({
        receiverId: config.worldContractId,
        actions: [actions.functionCall(method, args, GAS_100_TGAS, 0n)],
        blockHeightTtl: DELEGATE_TTL_BLOCKS,
      });
      const ok = await relay(toBase64(encodeSignedDelegate(signedDelegate)));
      return { txHash: ok.txHash, via: 'relayer', result: ok.result };
    } catch (e) {
      if (e instanceof RelayerRefused) {
        // The relayer understood the request and said no, or the contract call failed: do not retry directly.
        throw e.code === 'call_failed' ? new CallFailed(failureMessage(e.message), e.txHash) : e;
      }
      if (!(e instanceof RelayerUnavailable)) throw e;
      // Relayer down or overloaded: fall through to a direct call with the player's own key.
    }
  }

  const outcome = await account.callFunctionRaw({
    contractId: config.worldContractId,
    methodName: method,
    args,
    gas: GAS_100_TGAS,
    deposit: 0n,
    waitUntil: 'EXECUTED_OPTIMISTIC',
  });
  const txHash = outcome.transaction_outcome.id;
  const failure = failureOf(outcome);
  if (failure) throw new CallFailed(failureMessage(failure), txHash);
  let result: unknown = null;
  try {
    result = getTransactionLastResult(outcome);
  } catch {
    result = null;
  }
  return { txHash, via: 'direct', result };
}
