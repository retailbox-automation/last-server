export interface AppConfig {
  rpcUrl: string;
  worldContractId: string;
  /** Empty string = no relayer: players always send their own transactions. */
  relayerUrl: string;
  parentAccountId: string;
  explorerTxBase: string;
}

type Env = Record<string, string | boolean | undefined>;

export function readConfig(env: Env): AppConfig {
  const str = (k: string): string => {
    const v = env[k];
    return typeof v === 'string' ? v.trim() : '';
  };
  const relayer = env.VITE_RELAYER_URL === undefined ? (env.DEV === true ? 'http://localhost:8787' : '') : str('VITE_RELAYER_URL');
  return {
    rpcUrl: str('VITE_RPC_URL') || 'https://test.rpc.fastnear.com',
    worldContractId: str('VITE_WORLD_CONTRACT_ID') || 'world.lastserver.testnet',
    relayerUrl: relayer.replace(/\/+$/, ''),
    parentAccountId: str('VITE_PARENT_ACCOUNT_ID') || 'lastserver.testnet',
    explorerTxBase: 'https://testnet.nearblocks.io/txns/',
  };
}

export const config: AppConfig = readConfig((import.meta.env ?? {}) as Env);

/** Same rule as the relayer: 3-24 chars, lowercase letters, digits and "-", no "--", not reserved. */
const NAME_RE = /^[a-z0-9][a-z0-9-]{1,22}[a-z0-9]$/;
const RESERVED = new Set(['world', 'world-demo', 'lastserver', 'relayer', 'studio', 'admin', 'root', 'system']);

export function nameProblem(name: string): string | null {
  if (!NAME_RE.test(name) || name.includes('--')) {
    return 'Use 3-24 characters: lowercase letters, digits and "-" (no spaces).';
  }
  if (RESERVED.has(name)) return 'This name is reserved.';
  return null;
}
