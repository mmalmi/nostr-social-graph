import { mkdir, mkdtemp, readFile, rename, rm, rmdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DEFAULT_GRAPH_SNAPSHOT } from '../src/hashtreeSnapshot';
import { readMobileGraphSnapshot, verifyMobileGraphSnapshot, type MobileGraphReceipt } from '../src/mobileGraphSnapshot';

const dataDir = fileURLToPath(new URL('../data/', import.meta.url));
const binary = join(dataDir, 'socialGraph.bin');
const receiptPath = join(dataDir, 'socialGraph.json');
const root = '336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b';

function canonical(receipt: MobileGraphReceipt) {
  if (receipt.root !== root || receipt.source?.publisher !== DEFAULT_GRAPH_SNAPSHOT.publisher ||
      receipt.source?.treeName !== DEFAULT_GRAPH_SNAPSHOT.treeName) throw new Error('Unexpected bundled graph publisher, directory or scoring root');
}

async function refresh() {
  const previous = JSON.parse(await readFile(receiptPath, 'utf8'));
  const {bytes, receipt} = await readMobileGraphSnapshot(root);
  canonical(receipt);
  if (previous.version === 2 && (receipt.source.graphCompletedAt < previous.source.graphCompletedAt || receipt.source.generatedAt < previous.source.generatedAt)) {
    throw new Error('Signed graph crawl predates the pinned generation');
  }
  const stage = await mkdtemp(join(dataDir, '.graph-'));
  try {
    const oldBytes = await readFile(binary);
    await writeFile(join(stage, 'previous'), oldBytes);
    await writeFile(join(stage, 'bytes'), bytes);
    await writeFile(join(stage, 'receipt'), JSON.stringify(receipt, null, 2) + '\n');
    await rename(join(stage, 'bytes'), binary);
    try { await rename(join(stage, 'receipt'), receiptPath); }
    catch (error) { await rename(join(stage, 'previous'), binary); throw error; }
  } finally { await rm(stage, {recursive: true, force: true}); }
  return receipt;
}

async function main() {
  const [mode = '--verify', input, ...extra] = process.argv.slice(2);
  if (extra.length || !['--verify','--refresh'].includes(mode) || (mode === '--verify' && input)) {
    throw new Error('Usage: updateSocialGraph.ts [--verify | --refresh [pinned-member.bin]]');
  }
  const lock = join(dataDir, '.snapshot-refresh.lock');
  if (mode === '--refresh') await mkdir(lock);
  try {
    await run(mode, input);
  } finally {
    if (mode === '--refresh') await rmdir(lock);
  }
}

async function run(mode: string, input?: string) {
  let receipt: MobileGraphReceipt;
  if (mode === '--refresh' && !input) {
    receipt = await refresh();
  } else {
    receipt = JSON.parse(await readFile(receiptPath, 'utf8'));
    canonical(receipt);
    const bytes = await readFile(input ?? binary);
    await verifyMobileGraphSnapshot(bytes, receipt);
    if (input) {
      const stage = await mkdtemp(join(dataDir, '.graph-'));
      try {
        await writeFile(join(stage, 'bytes'), bytes);
        await rename(join(stage, 'bytes'), binary);
      } finally { await rm(stage, {recursive: true, force: true}); }
    }
  }
  console.log(`Verified discovery graph: ${receipt.users} users, ${receipt.bytes} bytes, ${receipt.source.eventId}`);
}

main().catch(error => { console.error(error instanceof Error ? error.message : error); process.exitCode = 1; });
