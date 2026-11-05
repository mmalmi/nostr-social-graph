import { expect, test } from '@playwright/test';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, preview } from 'vite';
import { WebSocketServer } from 'ws';
import { finalizeEvent, getPublicKey } from 'nostr-tools';
import { SocialGraph } from '../src/SocialGraph';
import { Negentropy, NegentropyStorageVector } from '../src/vendor/negentropy.js';

const packageRoot = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
const fixtureRoot = path.join(packageRoot, 'e2e/fixtures/publicGraphBootstrap');
const dist = path.resolve(process.env.GRAPH_BOOTSTRAP_DIST ?? path.join(packageRoot, 'dist'));
const secret = new Uint8Array(32).fill(7), root = getPublicKey(secret);
const friend = getPublicKey(new Uint8Array(32).fill(8));
const event = (kind: number, created_at: number) => finalizeEvent({ kind, created_at,
  content: '', tags: [['p', friend]] }, secret);

async function relay(events: ReturnType<typeof event>[], unsupported = false) {
  const server = new WebSocketServer({ port: 0 });
  await new Promise<void>(resolve => server.once('listening', resolve));
  const frames: any[][] = [];
  server.on('connection', socket => {
    const sessions = new Map<string, Negentropy>();
    socket.on('message', async raw => {
      const frame = JSON.parse(raw.toString());
      frames.push(frame);
      if (frame[0] === 'NEG-OPEN') {
        if (unsupported) { socket.send(JSON.stringify(['NEG-ERR', frame[1], 'unsupported'])); return; }
        const storage = new NegentropyStorageVector();
        for (const item of events) if (frame[2].authors.includes(item.pubkey) && frame[2].kinds.includes(item.kind)) {
          storage.insert(item.created_at, item.id);
        }
        storage.seal();
        const ne = new Negentropy(storage, 16384);
        sessions.set(frame[1], ne);
        socket.send(JSON.stringify(['NEG-MSG', frame[1], (await ne.reconcile(frame[3]))[0]]));
      } else if (frame[0] === 'NEG-MSG') {
        socket.send(JSON.stringify(['NEG-MSG', frame[1], (await sessions.get(frame[1])!.reconcile(frame[2]))[0]]));
      } else if (frame[0] === 'REQ') {
        // Respond to a broad query too: the baseline must visibly overwrite its
        // retained lists rather than pass because the fixture withheld bodies.
        for (const item of events) if (!frame[2].ids || frame[2].ids.includes(item.id)) {
          socket.send(JSON.stringify(['EVENT', frame[1], item]));
        }
        socket.send(JSON.stringify(['EOSE', frame[1]]));
      }
    });
  });
  return {
    frames,
    url: `ws://127.0.0.1:${(server.address() as { port: number }).port}`,
    close: () => new Promise<void>(resolve => {
      for (const socket of server.clients) socket.terminate();
      server.close(() => resolve());
    }),
  };
}

for (const entry of ['index.html', 'worker.html']) {
  test(`requires Negentropy and persists exact-ID catch-up in built ${entry}`, async ({ page }) => {
    const graph = new SocialGraph(root);
    graph.handleEvent([event(3, 100), event(10000, 101)]);
    const binary = [...await graph.toBinary()];
    const follow = event(3, 200), mute = event(10000, 201);
    const bad = await relay([follow, mute], true), good = await relay([follow, mute]);
    const outDir = await mkdtemp(path.join(os.tmpdir(), 'graph-sync-bundle-'));
    let server: Awaited<ReturnType<typeof preview>> | undefined;
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    try {
      await build({ configFile: false, root: fixtureRoot, logLevel: 'warn',
        resolve: { alias: [
          { find: /^nostr-social-graph\/publicGraphSync$/, replacement: path.join(dist, 'publicGraphSync.js') },
          { find: /^nostr-social-graph$/, replacement: path.join(dist, 'nostr-social-graph.es.js') },
        ] },
        build: { outDir, emptyOutDir: true, rollupOptions: { input: path.join(fixtureRoot, entry) } },
      });
      server = await preview({ configFile: false, root: fixtureRoot, logLevel: 'warn', build: { outDir },
        preview: { host: '127.0.0.1', port: 0 } });
      const address = server.httpServer.address();
      if (!address || typeof address === 'string') throw new Error('Missing preview address');
      await page.goto(`http://127.0.0.1:${address.port}/${entry}`);
      await page.waitForFunction(() => typeof (window as any).syncRetained === 'function');
      const sync = (relays: string[], retained = binary) => page.evaluate(options => (window as any).syncRetained(options),
        { root, binary: retained, relays });
      const failed = await sync([bad.url]);
      expect(failed.stats).toMatchObject({ failedRelays: 1, completedAuthors: 0, receivedEvents: 0, fallbackBatches: 0 });
      expect(failed.binary).toEqual(binary);
      expect(bad.frames.filter(frame => frame[0] === 'REQ')).toEqual([]);

      const recovered = await sync([bad.url, good.url], failed.binary);
      expect(recovered.stats).toMatchObject({ failedRelays: 1, completedAuthors: 1, receivedEvents: 2, fallbackBatches: 0 });
      expect(recovered.followRevision).toMatchObject({ id: follow.id, complete: true });
      expect(recovered.muteRevision).toMatchObject({ id: mute.id, complete: true });
      const requests = good.frames.filter(frame => frame[0] === 'REQ');
      expect(requests.flatMap(frame => frame[2].ids).sort()).toEqual([follow.id, mute.id].sort());
      expect(requests.every(frame => Object.keys(frame[2]).join(',') === 'ids')).toBe(true);
      expect(bad.frames.filter(frame => frame[0] === 'REQ')).toEqual([]);
      const repeated = await sync([good.url], recovered.binary);
      expect(repeated.stats).toMatchObject({ failedRelays: 0, completedAuthors: 1, receivedEvents: 0 });
      expect(repeated.binary).toEqual(recovered.binary);
      expect(errors).toEqual([]);
    } finally {
      await bad.close(); await good.close();
      if (server) await new Promise<void>((resolve, reject) => server!.httpServer.close(error => error ? reject(error) : resolve()));
      await rm(outDir, { recursive: true, force: true });
    }
  });
}
