import { once } from 'node:events';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { createNamedGraphFixture } from '../../src/testing/namedGraph';
import { WebSocketServer } from 'ws';

export async function namedGraphFixture(root: string, mobile: Uint8Array) {
  const {publisher, readBlock, profile, manifest, uploaded, signedTree, ...initial} = await createNamedGraphFixture(root, mobile);
  let event = initial.event;
  const requests: string[] = [];
  const filters: Record<string, unknown>[] = [];
  const server = createServer(async (req, res) => {
    res.setHeader('access-control-allow-origin','*');
    const hash = req.url?.match(/^\/([0-9a-f]{64})\.bin$/)?.[1];
    if (req.method !== 'GET' || !hash) { res.writeHead(404).end(); return; }
    requests.push(hash);
    const bytes = await readBlock(hash);
    if (bytes) res.writeHead(200, {'content-length': bytes.length}).end(bytes);
    else res.writeHead(404).end();
  });
  const relay = new WebSocketServer({server});
  relay.on('connection', socket => socket.on('message', raw => {
    const [op, sub, filter] = JSON.parse(raw.toString());
    if (op !== 'REQ') return;
    filters.push(filter);
    socket.send(JSON.stringify(['EVENT',sub,event]));
    socket.send(JSON.stringify(['EOSE',sub]));
  }));
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const host = '127.0.0.1:' + (server.address() as AddressInfo).port;
  return {
    options: {publisher, treeName: 'social-graph', relays: ['ws://' + host], blossomServers: ['http://' + host]},
    profile, requests, filters, manifest, uploaded,
    resign: async () => { event = await signedTree(); },
    corruptSignature: () => { event = {...event, sig: '0'.repeat(128)}; },
    close: async () => {
      for (const ws of relay.clients) ws.terminate();
      relay.close();
      server.closeAllConnections();
      await new Promise<void>(resolve => server.close(() => resolve()));
    },
  };
}
