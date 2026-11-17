import { readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';

function text(path) { try { return readFileSync(path, 'utf8'); } catch { return ''; } }
function kib(file, key) { const match = new RegExp('^' + key + ':\\s*(\\d+) kB$', 'm').exec(file); return match ? Number(match[1]) * 1024 : 0; }
function cgroupPath() {
  const leaf = text('/proc/self/cgroup').split('\n').find(line => line.startsWith('0::'))?.slice(3);
  return leaf && !leaf.includes('..') ? '/sys/fs/cgroup' + leaf : null;
}
export function save(path, value) {
  writeFileSync(path + '.tmp', JSON.stringify(value, null, 2) + '\n');
  renameSync(path + '.tmp', path);
}
export function monitor(pid, directory, label, limitBytes) {
  const started = performance.now(), group = cgroupPath(), stages = [], settled = [];
  let stage, risk, previousSave = 0;
  const path = directory + '/' + label + '-memory.json';
  const report = () => ({ pid, label, samplesEveryMs: 200, limitBytes, clock: 'monotonic', stages, settled, risk });
  function sample() {
    const status = text(`/proc/${pid}/status`), rss = kib(status, 'VmRSS');
    if (!rss || !stage) return null;
    const smaps = text(`/proc/${pid}/smaps_rollup`), hwm = kib(status, 'VmHWM');
    let helpers = 0;
    for (const id of readdirSync('/proc').filter(id => /^\d+$/.test(id) && Number(id) !== pid)) {
      const stat = text(`/proc/${id}/stat`), fields = stat.slice(stat.lastIndexOf(') ') + 2).split(' ');
      if (Number(fields[2]) === pid) helpers += kib(text(`/proc/${id}/status`), 'VmRSS');
    }
    const values = { rss, hwm, anon: kib(status, 'RssAnon'), pss: kib(smaps, 'Pss'), helpers, groupRss: rss + helpers };
    if (group) { const raw = text(group + '/memory.current').trim(); if (/^\d+$/.test(raw)) values.wholeCgroup = Number(raw); }
    stage.samples++;
    for (const [k,v] of Object.entries(values)) stage.peak[k] = Math.max(stage.peak[k] ?? 0, v);
    stage.last = values;
    const now = performance.now();
    if (!risk && (rss > limitBytes || kib(text('/proc/meminfo'), 'MemAvailable') < 768 * 2**20)) {
      risk = { reason: 'process limit or low memory', rss, limitBytes, atMs: now - started };
      save(path, report());
      try { process.kill(-pid, 'SIGTERM'); } catch {}
    }
    if (now - previousSave >= 3000) { save(path, report()); previousSave = now; }
    return values;
  }
  function mark(name) {
    sample(); const now = performance.now() - started;
    if (stage) stage.endMs = now;
    stage = { name, startMs: now, samples: 0, peak: {} }; stages.push(stage); sample();
  }
  mark(label + '-startup'); const timer = setInterval(sample, 200);
  return { mark, checkpoint(cycle) { const v = sample(); if (v) settled.push({cycle, atMs:performance.now()-started,...v}); save(path,report()); },
    finish() { clearInterval(timer); sample(); stage.endMs = performance.now() - started; save(path, report()); return report(); },
    check() { if (risk) throw Error('Runtime memory guard triggered: ' + JSON.stringify(risk)); } };
}
