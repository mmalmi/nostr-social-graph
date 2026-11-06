// Unsigned transport checks run before Playwright loads any graph fixture.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { createRequire } = require('node:module');
const { spawnSync } = require('node:child_process');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../../..');
const deps = createRequire(path.join(root, 'ts/package.json'));
const { chromium } = deps('@playwright/test');
const { WebSocketServer } = deps('ws');
const evidence = process.env.GRAPH_GATE_EVIDENCE;
const record = { passed: false, node: process.version,
  playwright: deps('@playwright/test/package.json').version };
const save = () => fs.writeFileSync(path.join(evidence, 'browser-runtime.json'), JSON.stringify(record, null, 2) + '\n');
const digest = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function distManifest() {
  const manifest = {};
  function visit(relative) {
    for (const entry of fs.readdirSync(path.join(root, 'ts/dist', relative), { withFileTypes: true })) {
      const name = path.posix.join(relative, entry.name);
      if (entry.isDirectory()) visit(name);
      else manifest[name] = digest(path.join(root, 'ts/dist', name));
    }
  }
  visit(''); return manifest;
}

async function unsignedProof() {
  assert.equal(process.version, 'v24.17.0');
  assert.equal(record.playwright, '1.61.1');
  assert.notEqual(process.getuid(), 0);
  const proof = spawnSync('python3', [path.join(__dirname, 'network-proof.py'),
    path.join(evidence, 'browser-network.json')], { stdio: 'inherit', timeout: 15000 });
  assert.equal(proof.status, 0, proof.error?.message);
  const build = JSON.parse(fs.readFileSync(path.join(evidence, 'build-runtime.json')));
  assert.equal(build.node, 'v22.23.4');
  assert.equal(build.passed, true);
  assert.deepEqual(distManifest(), build.productionDist, 'Production dist changed');
  record.productionBuild = { node: build.node, dist: build.productionDist };
  const server = http.createServer((_req, res) => res.end('<!doctype html><title>Unsigned graph boundary proof</title>'));
  const ws = new WebSocketServer({ server });
  ws.on('connection', socket => socket.on('message', bytes => socket.send(bytes.toString())));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch();
    record.chromium = browser.version();
    assert.equal(record.chromium, '149.0.7827.55');
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    record.rawWorker = await page.evaluate(async () => {
      // No transport shim, route interception, proxy, or signed content.
      const source = 'const ECHO_URL=' + JSON.stringify('ws://' + location.host + '/unsigned-echo') + ';' + String.raw`(async()=>{
        const outcomes=[];
        for(const host of ['203.0.113.1','[2001:db8::1]']){
          try{await fetch('https://'+host+'/unsigned-proof');outcomes.push({host,https:'escaped'});}
          catch(error){outcomes.push({host,https:'rejected',error:error.name});}
          outcomes.push(await new Promise(resolve=>{
            const socket=new WebSocket('wss://'+host+'/unsigned-proof');let failed=false;
            socket.onerror=()=>{failed=true;};
            socket.onopen=()=>{socket.close();resolve({host,wss:'escaped'});};
            socket.onclose=()=>resolve({host,wss:failed?'rejected':'close-without-error'});
          }));
        }
        const echo=await new Promise((resolve,reject)=>{
          const socket=new WebSocket(ECHO_URL);
          socket.onerror=()=>reject(Error('Loopback WebSocket failed'));
          socket.onopen=()=>socket.send('unsigned-loopback');
          socket.onmessage=event=>{socket.close();resolve(event.data);};
        });
        postMessage({outcomes,echo});
      })().catch(error=>postMessage({error:String(error)}));`;
      return await new Promise((resolve, reject) => {
        const url = URL.createObjectURL(new Blob([source], { type: 'application/javascript' }));
        const worker = new Worker(url);
        const close = () => { worker.terminate(); URL.revokeObjectURL(url); };
        const timer = setTimeout(() => { close(); reject(Error('Unsigned worker proof timed out')); }, 10000);
        worker.onmessage = event => { clearTimeout(timer); close(); resolve(event.data); };
        worker.onerror = event => { clearTimeout(timer); close(); reject(Error(event.message)); };
      });
    });
    assert.equal(record.rawWorker.outcomes?.length, 4, JSON.stringify(record.rawWorker));
    assert(record.rawWorker.outcomes.every(item => item.https === 'rejected' || item.wss === 'rejected'));
    assert.equal(record.rawWorker.echo, 'unsigned-loopback');
  } finally {
    await browser?.close();
    for (const socket of ws.clients) socket.terminate();
    await new Promise(resolve => ws.close(resolve));
    await new Promise(resolve => server.close(resolve));
  }
}

(async () => {
  try {
    await unsignedProof();
    record.unsignedProofPassed = true;
    save();
    // Preserve both page and default-IIFE cases in both existing spec files.
    const result = spawnSync(process.execPath, [deps.resolve('@playwright/test/cli'), 'test',
      'e2e/publicGraphBootstrap.e2e.spec.ts', 'e2e/publicGraphSync.e2e.spec.ts'], {
      cwd: path.join(root, 'ts'), stdio: 'inherit', timeout: 360000,
    });
    record.testExitCode = result.status;
    assert.equal(result.status, 0, result.error?.message);
    assert.deepEqual(distManifest(), record.productionBuild.dist);
    record.passed = true;
  } catch (error) {
    record.error = String(error);
    process.exitCode = 1;
  } finally { save(); }
})();
