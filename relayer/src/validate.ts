// Policy for what the relayer is willing to pay for. Every rejection carries an HTTP status and a
// stable machine-readable code; the HTTP layer maps them one to one.

import { createHash } from 'node:crypto';
import {
  DelegateAction,
  KeyType,
  PublicKey,
  SCHEMA,
  Signature,
  actions,
  encodeDelegateAction,
  encodeSignedDelegate,
} from 'near-api-js';
import type { SignedDelegate } from 'near-api-js';
import { deserialize } from 'borsh';
import type { PolicyConfig } from './config.ts';

/** Gas ceiling per relayed call (spec: 100 Tgas). */
export const MAX_GAS = 100_000_000_000_000n;
export const ALLOWED_METHODS: ReadonlySet<string> = new Set(['start_dive', 'settle_dive']);
/** A signed delegate is a few hundred bytes plus the call arguments (settle_dive: up to 4000 moves). */
export const MAX_DELEGATE_BYTES = 16 * 1024;
export const MAX_ARGS_BYTES = 8 * 1024;

/** `^[a-z0-9][a-z0-9-]{1,22}[a-z0-9]$` from the spec, plus NEAR's rule that "-" never doubles. */
const NAME_RE = /^[a-z0-9][a-z0-9-]{1,22}[a-z0-9]$/;
/** Labels that would collide with infrastructure accounts under the studio account. */
const RESERVED_NAMES: ReadonlySet<string> = new Set([
  'world',
  'world-demo',
  'lastserver',
  'relayer',
  'studio',
  'admin',
  'root',
  'system',
]);

export class Rejection extends Error {
  readonly status: number;
  readonly code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'Rejection';
    this.status = status;
    this.code = code;
  }
}

/** Returns the reason a player name is not acceptable, or null when it is fine. */
export function playerNameProblem(name: unknown): string | null {
  if (typeof name !== 'string') return 'name must be a string';
  if (!NAME_RE.test(name)) {
    return 'name must be 3-24 characters: lowercase letters, digits and "-", starting and ending with a letter or digit';
  }
  if (name.includes('--')) return 'name must not contain "--"';
  if (RESERVED_NAMES.has(name)) return 'this name is reserved';
  return null;
}

/** `bob.lastserver.testnet` -> `bob`; null if the id is not a direct sub-account of the parent. */
export function playerLabel(accountId: string, parentId: string): string | null {
  const suffix = `.${parentId}`;
  if (!accountId.endsWith(suffix)) return null;
  const label = accountId.slice(0, -suffix.length);
  return playerNameProblem(label) === null ? label : null;
}

// Shapes produced by borsh's deserialize for SCHEMA.SignedDelegate (plain objects, numbers arrays).
interface DecodedFunctionCall {
  methodName: string;
  args: ArrayLike<number>;
  gas: bigint;
  deposit: bigint;
}
type DecodedAction = { functionCall?: DecodedFunctionCall } & Record<string, unknown>;
interface DecodedDelegate {
  delegateAction: {
    senderId: string;
    receiverId: string;
    actions: DecodedAction[];
    nonce: bigint;
    maxBlockHeight: bigint;
    publicKey: { ed25519Key?: { data: ArrayLike<number> } } & Record<string, unknown>;
  };
  signature: { ed25519Signature?: { data: ArrayLike<number> } } & Record<string, unknown>;
}

export interface ValidDelegate {
  /** Typed object ready to be wrapped into a transaction action. */
  signedDelegate: SignedDelegate;
  senderId: string;
  method: string;
  gas: bigint;
}

const BASE64_RE = /^[A-Za-z0-9+/]*={0,2}$/;

function decodeBase64(text: string): Uint8Array {
  if (text.length === 0 || text.length % 4 !== 0 || !BASE64_RE.test(text)) {
    throw new Rejection(400, 'bad_delegate', 'signedDelegate must be standard base64');
  }
  if (text.length > Math.ceil((MAX_DELEGATE_BYTES * 4) / 3) + 4) {
    throw new Rejection(400, 'delegate_too_large', 'signedDelegate is too large');
  }
  return new Uint8Array(Buffer.from(text, 'base64'));
}

/**
 * Decodes and checks a base64 borsh SignedDelegate (NEP-366). Throws Rejection:
 * 400 for undecodable input, 403 for anything the relayer is not allowed to pay for.
 */
export function validateSignedDelegate(base64: unknown, policy: PolicyConfig): ValidDelegate {
  if (typeof base64 !== 'string') throw new Rejection(400, 'bad_delegate', 'signedDelegate must be a base64 string');
  const bytes = decodeBase64(base64);
  if (bytes.length > MAX_DELEGATE_BYTES) {
    throw new Rejection(400, 'delegate_too_large', 'signedDelegate is too large');
  }

  let decoded: DecodedDelegate;
  try {
    decoded = deserialize(SCHEMA.SignedDelegate, bytes) as unknown as DecodedDelegate;
  } catch {
    throw new Rejection(400, 'bad_delegate', 'signedDelegate is not a valid borsh SignedDelegate');
  }
  const da = decoded.delegateAction;

  // Sender: a direct, non-reserved sub-account of the studio account.
  if (playerLabel(da.senderId, policy.parentId) === null) {
    throw new Rejection(403, 'wrong_sender', `sender must be <name>.${policy.parentId}`);
  }
  if (da.receiverId !== policy.worldContractId) {
    throw new Rejection(403, 'wrong_receiver', `receiver must be ${policy.worldContractId}`);
  }
  if (da.actions.length !== 1) {
    throw new Rejection(403, 'bad_actions', 'exactly one action is allowed');
  }
  const action = da.actions[0]!;
  const call = action.functionCall;
  if (!call) throw new Rejection(403, 'forbidden_action', 'only FunctionCall actions are allowed');
  if (!ALLOWED_METHODS.has(call.methodName)) {
    throw new Rejection(403, 'forbidden_method', 'only start_dive and settle_dive are allowed');
  }
  if (call.deposit !== 0n) throw new Rejection(403, 'deposit_not_allowed', 'deposit must be 0');
  if (call.gas > MAX_GAS) throw new Rejection(403, 'gas_too_high', 'gas must be at most 100 Tgas');
  if (call.gas === 0n) throw new Rejection(403, 'gas_zero', 'gas must be greater than 0');
  if (call.args.length > MAX_ARGS_BYTES) {
    throw new Rejection(403, 'args_too_large', `call arguments must be at most ${MAX_ARGS_BYTES} bytes`);
  }

  const pkData = da.publicKey.ed25519Key?.data;
  const sigData = decoded.signature.ed25519Signature?.data;
  if (!pkData || !sigData) {
    throw new Rejection(403, 'bad_signature', 'only ed25519 keys and signatures are accepted');
  }
  const publicKey = new PublicKey({ keyType: KeyType.ED25519, data: Uint8Array.from(pkData) });
  const signatureBytes = Uint8Array.from(sigData);

  // Rebuild typed objects; the canonical re-encoding must equal the input (no trailing or odd bytes).
  const delegateAction = new DelegateAction({
    senderId: da.senderId,
    receiverId: da.receiverId,
    actions: [actions.functionCall(call.methodName, Uint8Array.from(call.args), call.gas, call.deposit)],
    nonce: da.nonce,
    maxBlockHeight: da.maxBlockHeight,
    publicKey,
  });
  // near-api-js declares the SignedDelegate class but does not export it at runtime; a plain object
  // with the same fields is what its encoder and `actions.signedDelegate` take.
  const signedDelegate: SignedDelegate = {
    delegateAction,
    signature: new Signature({ keyType: publicKey.keyType, data: signatureBytes }),
  };
  const canonical = encodeSignedDelegate(signedDelegate);
  if (canonical.length !== bytes.length || !canonical.every((b, i) => b === bytes[i])) {
    throw new Rejection(400, 'bad_delegate', 'signedDelegate is not in canonical borsh form');
  }

  // A forged or corrupted signature makes the chain reject the delegate after the relayer paid for it.
  const hash = createHash('sha256').update(encodeDelegateAction(delegateAction)).digest();
  let signatureOk = false;
  try {
    signatureOk = publicKey.verify(new Uint8Array(hash), signatureBytes);
  } catch {
    signatureOk = false;
  }
  if (!signatureOk) throw new Rejection(403, 'bad_signature', 'delegate signature does not verify');

  return { signedDelegate, senderId: da.senderId, method: call.methodName, gas: call.gas };
}
