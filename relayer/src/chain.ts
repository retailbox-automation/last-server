// Everything that talks to NEAR. The HTTP layer depends only on the `Chain` interface, so tests
// run against a fake and never touch a network.

import {
  Account,
  JsonRpcProvider,
  KeyPair,
  KeyPairSigner,
  PublicKey,
  actions,
  formatNearAmount,
  parseNearAmount,
} from 'near-api-js';
import type { SignedDelegate } from 'near-api-js';
import { AccessKeyDoesNotExistError, AccountDoesNotExistError } from 'near-api-js/rpc-errors';
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

export interface Health {
  blockHeight: number;
  /** Studio account balance in NEAR, as a decimal string. */
  balanceNear: string;
}

export class ChainError extends Error {
  readonly code: 'account_exists' | 'chain_error';
  constructor(code: 'account_exists' | 'chain_error', message: string) {
    super(message);
    this.name = 'ChainError';
    this.code = code;
  }
}

export interface Chain {
  accountStatus(accountId: string, publicKey: string): Promise<AccountStatus>;
  createPlayer(accountId: string, publicKey: string): Promise<{ txHash: string }>;
  relay(signedDelegate: SignedDelegate, worldContractId: string): Promise<RelayResult>;
  health(): Promise<Health>;
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

export class NearChain implements Chain {
  private readonly account: Account;
  private readonly provider: JsonRpcProvider;
  private readonly fundingYocto: bigint;

  constructor(config: Config) {
    this.provider = new JsonRpcProvider({ url: config.rpcUrl });
    const signer = new KeyPairSigner(KeyPair.fromString(config.secretKey as `ed25519:${string}`));
    this.account = new Account(config.parentId, this.provider, signer);
    const yocto = parseNearAmount(config.signupFundingNear as `${number}`);
    if (yocto === null) throw new Error('SIGNUP_FUNDING_NEAR is not a valid NEAR amount');
    this.fundingYocto = BigInt(yocto);
  }

  async accountStatus(accountId: string, publicKey: string): Promise<AccountStatus> {
    try {
      await this.provider.viewAccessKey({ accountId, publicKey: PublicKey.fromString(publicKey) });
      return 'has_key';
    } catch (e) {
      if (e instanceof AccountDoesNotExistError) return 'absent';
      if (e instanceof AccessKeyDoesNotExistError) return 'other_key';
      throw new ChainError('chain_error', `account lookup failed: ${String((e as Error).message ?? e)}`);
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
    try {
      const outcome = await this.account.signAndSendTransaction({
        receiverId: signedDelegate.delegateAction.senderId,
        actions: [actions.signedDelegate(signedDelegate)],
        throwOnFailure: false,
        waitUntil: 'EXECUTED_OPTIMISTIC',
      });
      return summarizeOutcome(outcome as unknown as OutcomeLike, worldContractId);
    } catch (e) {
      throw new ChainError('chain_error', `relay failed: ${String((e as Error).message ?? e)}`);
    }
  }

  async health(): Promise<Health> {
    try {
      const [block, state] = await Promise.all([
        this.provider.viewBlock({ finality: 'final' }),
        this.provider.viewAccount({ accountId: this.account.accountId, blockQuery: { finality: 'final' } }),
      ]);
      return {
        blockHeight: Number(block.header.height),
        balanceNear: formatNearAmount(state.amount.toString(), 2),
      };
    } catch (e) {
      throw new ChainError('chain_error', `health check failed: ${String((e as Error).message ?? e)}`);
    }
  }
}
