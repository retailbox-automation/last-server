import {
  DelegateAction,
  KeyPair,
  KeyPairSigner,
  actions,
  encodeSignedDelegate,
} from 'near-api-js';
import type { Action, SignedDelegate } from 'near-api-js';
import { DEFAULT_LIMITS } from '../src/config.ts';
import type { PolicyConfig } from '../src/config.ts';

export const PARENT = 'lastserver.testnet';
export const WORLD = 'world.lastserver.testnet';
export const NEAR = 10n ** 24n;
/** Chain height the fake chain reports; DEFAULT_MAX_BLOCK_HEIGHT is 100 blocks past it, like the web. */
export const HEIGHT = 999_900n;
export const DEFAULT_MAX_BLOCK_HEIGHT = 1_000_000n;

export function policy(over: Partial<PolicyConfig> = {}): PolicyConfig {
  return {
    parentId: PARENT,
    worldContractId: WORLD,
    allowedOrigin: '*',
    trustProxyHops: 0,
    signupFundingYocto: NEAR / 5n,
    minStudioBalanceYocto: NEAR,
    limits: { ...DEFAULT_LIMITS },
    ...over,
  };
}

/** Throwaway ed25519 key; nothing here ever touches a network. */
export function throwawayKey(): KeyPair {
  return KeyPair.fromRandom('ed25519');
}

export interface DelegateSpec {
  sender?: string;
  receiver?: string;
  method?: string;
  args?: Uint8Array | object;
  gas?: bigint;
  deposit?: bigint;
  actions?: Action[];
  /** Key that signs (default: a fresh throwaway key, also used as the delegate's public key). */
  key?: KeyPair;
  /** Public key written into the delegate action if different from the signer's. */
  publicKeyOf?: KeyPair;
  nonce?: bigint;
  maxBlockHeight?: bigint;
}

export async function signDelegate(spec: DelegateSpec = {}): Promise<SignedDelegate> {
  const key = spec.key ?? throwawayKey();
  const signer = new KeyPairSigner(key);
  const callActions = spec.actions ?? [
    actions.functionCall(
      spec.method ?? 'start_dive',
      spec.args ?? { loadout: [] },
      spec.gas ?? 100_000_000_000_000n,
      spec.deposit ?? 0n,
    ),
  ];
  const delegateAction = new DelegateAction({
    senderId: spec.sender ?? `bob.${PARENT}`,
    receiverId: spec.receiver ?? WORLD,
    actions: callActions,
    nonce: spec.nonce ?? 7n,
    maxBlockHeight: spec.maxBlockHeight ?? DEFAULT_MAX_BLOCK_HEIGHT,
    publicKey: (spec.publicKeyOf ?? key).getPublicKey(),
  });
  if (spec.publicKeyOf) {
    // Sign with `key` although the action names another public key: the signer refuses that, so sign by hand.
    const { createHash } = await import('node:crypto');
    const { encodeDelegateAction, Signature } = await import('near-api-js');
    const hash = new Uint8Array(createHash('sha256').update(encodeDelegateAction(delegateAction)).digest());
    const sig = key.sign(hash).signature;
    return { delegateAction, signature: new Signature({ keyType: 0, data: sig }) };
  }
  return (await signer.signDelegateAction(delegateAction)).signedDelegate;
}

export async function delegateBase64(spec: DelegateSpec = {}): Promise<string> {
  return Buffer.from(encodeSignedDelegate(await signDelegate(spec))).toString('base64');
}
