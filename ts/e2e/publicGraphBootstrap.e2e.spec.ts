import { expect, test } from '@playwright/test';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, preview } from 'vite';
import { SocialGraph } from '../src/SocialGraph';

const packageRoot = path.resolve(fileURLToPath(new URL('..', import.meta.url)));
const fixtureRoot = path.join(packageRoot, 'e2e/fixtures/publicGraphBootstrap');
// An extracted release can be supplied to demonstrate the regression against
// immutable package bytes, without replacing the working tree or its dist.
const dist = path.resolve(process.env.GRAPH_BOOTSTRAP_DIST ?? path.join(packageRoot, 'dist'));
const root = '1'.repeat(64);

for (const entry of ['index.html', 'worker.html']) {
  test(`bootstraps and retains follow/mute revisions in built ${entry}`, async ({ page }) => {
    const graph = new SocialGraph(root);
    graph.handleEvent({ kind: 3, pubkey: root, created_at: 20, id: 'a'.repeat(64), tags: [['p', '2'.repeat(64)]], content: '', sig: '' });
    graph.handleEvent({ kind: 10000, pubkey: root, created_at: 21, id: 'b'.repeat(64), tags: [['p', '3'.repeat(64)]], content: '', sig: '' });
    graph.markListIncomplete(root, 3);
    graph.markListIncomplete(root, 10000);
    const body = Buffer.from(await graph.toBinary());
    const outDir = await mkdtemp(path.join(os.tmpdir(), 'graph-bootstrap-bundle-'));
    let server: Awaited<ReturnType<typeof preview>> | undefined;
    try {
      await build({
        configFile: false,
        root: fixtureRoot,
        logLevel: 'warn',
        resolve: { alias: [
          { find: /^nostr-social-graph\/publicGraphSync$/, replacement: path.join(dist, 'publicGraphSync.js') },
          { find: /^nostr-social-graph$/, replacement: path.join(dist, 'nostr-social-graph.es.js') },
        ] },
        // Deliberately retain Vite's default IIFE worker format. Requiring
        // consumers to change their worker output would hide a packaging bug.
        build: { outDir, emptyOutDir: true, rollupOptions: { input: path.join(fixtureRoot, entry) } },
      });
      server = await preview({
        configFile: false,
        root: fixtureRoot,
        logLevel: 'warn',
        build: { outDir },
        preview: { host: '127.0.0.1', port: 0 },
        plugins: [{ name: 'graph-snapshot-fixture', configurePreviewServer(previewServer) {
          previewServer.middlewares.use((req, res, next) => {
            if (!req.url?.startsWith('/snapshot.bin?')) return next();
            expect(new URL(req.url, 'http://localhost').searchParams.get('root')).toBe(root);
            res.setHeader('Content-Type', 'application/octet-stream');
            res.setHeader('x-graph-root', root);
            res.end(body);
          });
        } }],
      });
      const address = server.httpServer.address();
      if (!address || typeof address === 'string') throw new Error('Missing preview address');
      const base = `http://127.0.0.1:${address.port}`;
      await page.goto(`${base}/${entry}`);
      await page.waitForFunction(() => typeof (window as any).bootstrap === 'function');
      const loaded = await page.evaluate(url => (window as any).bootstrap(url), `${base}/snapshot.bin`);
      expect(loaded).toEqual({
        root,
        follows: ['2'.repeat(64)],
        mutes: ['3'.repeat(64)],
        followRevision: { id: 'a'.repeat(64), created_at: 20, complete: false },
        muteRevision: { id: 'b'.repeat(64), created_at: 21, complete: false },
      });
    } finally {
      if (server) await new Promise<void>((resolve, reject) => server!.httpServer.close(error => error ? reject(error) : resolve()));
      await rm(outDir, { recursive: true, force: true });
    }
  });
}
