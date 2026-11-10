// Relayer configuration. Secrets come only from the environment and never reach logs or responses.

import { parseNearAmount } from 'near-api-js';

export interface Limits {
  relaySenderMax: number;
  relaySenderWindowS: number;
  relayIpMax: number;
  relayIpWindowS: number;
  /** All relays together, whatever the sender or IP: caps what the studio pays for gas in a window. */
  relayGlobalMax: number;
  relayGlobalWindowS: number;
  signupIpMax: number;
  signupIpWindowS: number;
  signupGlobalMax: number;
  signupGlobalWindowS: number;
  /** Cheap reads that still cost an RPC call: GET /health and the name check of POST /signup. */
  lookupIpMax: number;
  lookupIpWindowS: number;
}

/** Everything the HTTP layer needs; contains no secret. */
export interface PolicyConfig {
  /** Studio account; players are its direct sub-accounts. */
  parentId: string;
  /** The only contract the relayer forwards calls to. */
  worldContractId: string;
  allowedOrigin: string;
  /** How many reverse proxies sit in front of us; 0 = trust only the socket address. */
  trustProxyHops: number;
  /** NEAR each new player account receives (yocto). */
  signupFundingYocto: bigint;
  /** Signup is refused when it would leave the studio account with less than this (yocto). */
  minStudioBalanceYocto: bigint;
  limits: Limits;
}

export interface Config extends PolicyConfig {
  secretKey: string;
  rpcUrl: string;
  port: number;
  host: string;
}

export const DEFAULT_RPC_URL = 'https://test.rpc.fastnear.com';

export const DEFAULT_LIMITS: Limits = {
  relaySenderMax: 30,
  relaySenderWindowS: 60,
  relayIpMax: 120,
  relayIpWindowS: 60,
  // A relayed call burns well under 0.01 testnet NEAR of gas; 1000 a day bounds the studio's spend on relays.
  relayGlobalMax: 1000,
  relayGlobalWindowS: 86_400,
  signupIpMax: 5,
  signupIpWindowS: 3600,
  // 20 x 0.2 NEAR = 4 testnet NEAR a day; the balance floor below stops signup before the studio runs dry.
  signupGlobalMax: 20,
  signupGlobalWindowS: 86_400,
  lookupIpMax: 30,
  lookupIpWindowS: 60,
};

export const DEFAULT_SIGNUP_FUNDING_NEAR = '0.2';
export const DEFAULT_MIN_STUDIO_BALANCE_NEAR = '1';

function intFrom(env: NodeJS.ProcessEnv, name: string, fallback: number, problems: string[]): number {
  const raw = env[name];
  if (raw === undefined || raw === '') return fallback;
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 0) {
    problems.push(`${name} must be a non-negative integer`);
    return fallback;
  }
  return n;
}

function nearFrom(env: NodeJS.ProcessEnv, name: string, fallback: string, problems: string[]): bigint {
  const raw = env[name] || fallback;
  const yocto = /^\d+(\.\d+)?$/.test(raw) ? parseNearAmount(raw as `${number}`) : null;
  if (yocto === null) {
    problems.push(`${name} must be a NEAR amount like 0.2`);
    return 0n;
  }
  return BigInt(yocto);
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const problems: string[] = [];
  const required = (name: string): string => {
    const v = env[name];
    if (v === undefined || v === '') {
      problems.push(`${name} is required`);
      return '';
    }
    return v;
  };
  const parentId = required('LASTSERVER_ACCOUNT_ID');
  const secretKey = required('LASTSERVER_SECRET_KEY');
  const worldContractId = required('WORLD_CONTRACT_ID');
  const d = DEFAULT_LIMITS;
  const limits: Limits = {
    relaySenderMax: intFrom(env, 'RATE_RELAY_SENDER_MAX', d.relaySenderMax, problems),
    relaySenderWindowS: intFrom(env, 'RATE_RELAY_SENDER_WINDOW_S', d.relaySenderWindowS, problems),
    relayIpMax: intFrom(env, 'RATE_RELAY_IP_MAX', d.relayIpMax, problems),
    relayIpWindowS: intFrom(env, 'RATE_RELAY_IP_WINDOW_S', d.relayIpWindowS, problems),
    relayGlobalMax: intFrom(env, 'RATE_RELAY_GLOBAL_MAX', d.relayGlobalMax, problems),
    relayGlobalWindowS: intFrom(env, 'RATE_RELAY_GLOBAL_WINDOW_S', d.relayGlobalWindowS, problems),
    signupIpMax: intFrom(env, 'RATE_SIGNUP_IP_MAX', d.signupIpMax, problems),
    signupIpWindowS: intFrom(env, 'RATE_SIGNUP_IP_WINDOW_S', d.signupIpWindowS, problems),
    signupGlobalMax: intFrom(env, 'RATE_SIGNUP_GLOBAL_MAX', d.signupGlobalMax, problems),
    signupGlobalWindowS: intFrom(env, 'RATE_SIGNUP_GLOBAL_WINDOW_S', d.signupGlobalWindowS, problems),
    lookupIpMax: intFrom(env, 'RATE_LOOKUP_IP_MAX', d.lookupIpMax, problems),
    lookupIpWindowS: intFrom(env, 'RATE_LOOKUP_IP_WINDOW_S', d.lookupIpWindowS, problems),
  };
  const config: Config = {
    parentId,
    secretKey,
    worldContractId,
    rpcUrl: env.RPC_URL || DEFAULT_RPC_URL,
    port: intFrom(env, 'PORT', 8787, problems),
    host: env.HOST || '0.0.0.0',
    allowedOrigin: env.ALLOWED_ORIGIN || '*',
    trustProxyHops: intFrom(env, 'TRUST_PROXY_HOPS', 0, problems),
    signupFundingYocto: nearFrom(env, 'SIGNUP_FUNDING_NEAR', DEFAULT_SIGNUP_FUNDING_NEAR, problems),
    minStudioBalanceYocto: nearFrom(env, 'MIN_STUDIO_BALANCE_NEAR', DEFAULT_MIN_STUDIO_BALANCE_NEAR, problems),
    limits,
  };
  if (problems.length > 0) throw new Error(`Invalid relayer configuration: ${problems.join('; ')}`);
  return config;
}
