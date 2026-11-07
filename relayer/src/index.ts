// Entry point: `node dist/src/index.js` after `npm run build`, or `npm start` (runs the TypeScript directly).

import { NearChain } from './chain.ts';
import { createApp } from './app.ts';
import { loadConfig } from './config.ts';

const config = loadConfig();
const server = createApp({ policy: config, chain: new NearChain(config) });
server.listen(config.port, config.host, () => {
  console.log(
    JSON.stringify({
      ev: 'listening',
      host: config.host,
      port: config.port,
      parent: config.parentId,
      world: config.worldContractId,
      rpc: config.rpcUrl,
    }),
  );
});
