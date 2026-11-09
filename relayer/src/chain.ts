// Everything that talks to NEAR. The HTTP layer depends only on the `Chain` interface, so tests
// run against a fake and never touch a network.

import { AsyncLocalStorage } from 'node:async_hooks';
import { createHash } from 'node:crypto';
import {
  Account,
  JsonRpcProvider,
  KeyPair,
  KeyPairSigner,
  PublicKey,
  actions,
  baseEncode,
  encodeTransaction,
} from 'near-api-js';
import type { SignedDelegate } from 'near-api-js';
import {
  AccessKeyDoesNotExistError,
  AccountDoesNotExistError,
  ActionExecutionError,
  InvalidTransactionError,
  TransactionExecutionError,
} from 'near-api-js/rpc-errors';
import type { Config } from './config.ts';

export type AccountStatus = 'absent' | 'has_key' | 'other_key';

export interface RelayResult {
  txHash: string;
  /** True when the studio transaction and every receipt it spawned succeeded. */
  ok: boolean;
  /** Human-readable failure reason when ok is false. */
  failure?: string;
  /** Base64 of the contract call's return value (e.g. the DiveOutcome JSON), if any. */
  resultBase64?: string;
}

/**
 * - `account_exists`: the account to create is already there.
 * - `chain_error`: nothing reached the chain (or the chain rejected the transaction outright).
 * - `relay_unknown`: the transaction was handed to the RPC but its outcome is unknown (timeout,
 *   dropped connection); `txHash` names it, so the caller can look it up instead of resending.
 */
export type ChainErrorCode = 'account_exists' | 'chain_error' | 'relay_unknown';

export class ChainError extends Error {
  readonly code: ChainErrorCode;
  readonly txHash: string | undefined;
  constructor(code: ChainErrorCode, message: string, txHash?: string) {
    super(message);
    this.name = 'ChainError';
    this.code = code;
    this.txHash = txHash;
  }
}

export interface Chain {
  accountStatus(accountId: string, publicKey: string): Promise<AccountStatus>;
  createPlayer(accountId: string, publicKey: string): Promise<{ txHash: string }>;
  relay(signedDelegate: SignedDelegate, worldContractId: string): Promise<RelayResult>;
  /** Latest (optimistic) block height. */
  blockHeight(): Promise<bigint>;
  /** Studio account balance, yoctoNEAR. */
  studioBalance(): Promise<bigint>;
}

// Minimal view of the RPC transaction outcome we read.
interface OutcomeLike {
  status?: unknown;
  transaction_outcome?: { id?: string };
  receipts_outcome?: Array<{ outcome?: { executor_id?: string; status?: unknown } }>;
}

function failureText(status: unknown): string | null {
  if (status && typeof status === 'object' && 'Failure' in status) {
    const text = JSON.stringify((status as { Failure: unknown }).Failure);
    return text.length > 400 ? `${text.slice(0, 400)}...` : text;
  }
  return null;
}

function successValue(status: unknown): string | null {
  if (status && typeof status === 'object' && 'SuccessValue' in status) {
    const v = (status as { SuccessValue: unknown }).SuccessValue;
    if (typeof v === 'string' && v.length > 0) return v;
  }
  return null;
}

/**
 * Reads a relayed transaction's outcome: any failed receipt means failure; the return value is the
 * overall SuccessValue, or the one produced by the world contract's own receipt.
 */
export function summarizeOutcome(outcome: OutcomeLike, worldContractId: string): RelayResult {
  const txHash = outcome.transaction_outcome?.id ?? '';
  const failure =
    failureText(outcome.status) ??
    outcome.receipts_outcome?.map((r) => failureText(r.outcome?.status)).find((f) => f !== null) ??
    null;
  if (failure !== null) return { txHash, ok: false, failure };
  const fromContract = outcome.receipts_outcome
    ?.filter((r) => r.outcome?.executor_id === worldContractId)
    .map((r) => successValue(r.outcome?.status))
    .find((v) => v !== null);
  const resultBase64 = successValue(outcome.status) ?? fromContract ?? undefined;
  return resultBase64 === undefined ? { txHash, ok: true } : { txHash, ok: true, resultBase64 };
}

/** Hash of the transaction the current `relay()` call handed to the RPC, if it got that far. */
const sendScope = new AsyncLocalStorage<{ txHash?: string }>();

/** Records the hash of every transaction just before it is sent, for the `relay()` call in scope. */
class TrackingProvider extends JsonRpcProvider {
  override async sendTransactionUntil(
    ...args: Parameters<JsonRpcProvider['sendTransactionUntil']>
  ): ReturnType<JsonRpcProvider['sendTransactionUntil']> {
    const scope = sendScope.getStore();
    if (scope) scope.txHash = baseEncode(createHash('sha256').update(encodeTransaction(args[0].transaction)).digest());
    return super.sendTransactionUntil(...args);
  }
}

/** A send that the chain refused to accept: nothing executed, so it is safe to try another way. */
function rejectedBeforeExecution(e: unknown): boolean {
  return e instanceof InvalidTransactionError || (e instanceof TransactionExecutionError && !(e instanceof ActionExecutionError));
}

/** RPC retries on 5xx/408 (near-api-js defaults to 12 with backoff, about two minutes); keep the relayer well under the web's 30 s. */
const RPC_RETRIES = 5;

export class NearChain implements Chain {
  private readonly account: Account;
  private readonly provider: JsonRpcProvider;
  private readonly fundingYocto: bigint;

  constructor(config: Config) {
    this.provider = new TrackingProvider({ url: config.rpcUrl }, { retries: RPC_RETRIES });
    const signer = new KeyPairSigner(KeyPair.fromString(config.secretKey as `ed25519:${string}`));
    this.account = new Account(config.parentId, this.provider, signer);
    this.fundingYocto = config.signupFundingYocto;
  }

  async accountStatus(accountId: string, publicKey: string): Promise<AccountStatus> {
    // view_access_key alone cannot tell a free name from a taken one: testnet answers "access key
    // ... does not exist" for an account that does not exist at all. Ask for the account first.
    try {
      await this.provider.viewAccount({ accountId, blockQuery: { finality: 'optimistic' } });
    } catch (e) {
      if (e instanceof AccountDoesNotExistError) return 'absent';
      throw new ChainError('chain_error', `account lookup failed: ${String((e as Error).message ?? e)}`);
    }
    try {
      await this.provider.viewAccessKey({
        accountId,
        publicKey: PublicKey.fromString(publicKey),
        finalityQuery: { finality: 'optimistic' },
      });
      return 'has_key';
    } catch (e) {
      if (e instanceof AccessKeyDoesNotExistError) return 'other_key';
      throw new ChainError('chain_error', `access key lookup failed: ${String((e as Error).message ?? e)}`);
    }
  }

  async createPlayer(accountId: string, publicKey: string): Promise<{ txHash: string }> {
    try {
      const r = await this.account.createSubAccount({
        accountOrPrefix: accountId,
        publicKey: PublicKey.fromString(publicKey),
        nearToTransfer: this.fundingYocto,
      });
      return { txHash: r.transaction_outcome.id };
    } catch (e) {
      const msg = String((e as Error).message ?? e);
      if (/already exists|AccountAlreadyExists/i.test(msg) || (e as Error).name === 'AccountAlreadyExistsActionError') {
        throw new ChainError('account_exists', 'account already exists');
      }
      throw new ChainError('chain_error', `account creation failed: ${msg}`);
    }
  }

  async relay(signedDelegate: SignedDelegate, worldContractId: string): Promise<RelayResult> {
    const scope: { txHash?: string } = {};
    try {
      const outcome = await sendScope.run(scope, () =>
        this.account.signAndSendTransaction({
          receiverId: signedDelegate.delegateAction.senderId,
          actions: [actions.signedDelegate(signedDelegate)],
          throwOnFailure: false,
          waitUntil: 'EXECUTED_OPTIMISTIC',
        }),
      );
      return summarizeOutcome(outcome as unknown as OutcomeLike, worldContractId);
    } catch (e) {
      const msg = String((e as Error).message ?? e);
      if (scope.txHash === undefined || rejectedBeforeExecution(e)) {
        throw new ChainError('chain_error', `relay failed: ${msg}`);
      }
      // Sent, but no outcome: it may still execute. Report the hash; never let the caller resend blindly.
      throw new ChainError('relay_unknown', `relay outcome unknown: ${msg}`, scope.txHash);
    }
  }

  async blockHeight(): Promise<bigint> {
    try {
      const block = await this.provider.viewBlock({ finality: 'optimistic' });
      return BigInt(block.header.height);
    } catch (e) {
      throw new ChainError('chain_error', `block height lookup failed: ${String((e as Error).message ?? e)}`);
    }
  }

  async studioBalance(): Promise<bigint> {
    try {
      const state = await this.provider.viewAccount({ accountId: this.account.accountId, blockQuery: { finality: 'optimistic' } });
      return state.amount;
    } catch (e) {
      throw new ChainError('chain_error', `balance lookup failed: ${String((e as Error).message ?? e)}`);
    }
  }
}
