// Aggregate-only memory sampling. Linux proportional set size avoids counting
// the same physical pages twice across a writer and a newly forked child.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';

const intervalMs = 100;

function read(file, optional = false) {
  try { return readFileSync(file, 'utf8'); }
  catch (error) {
    if (['ENOENT', 'ESRCH'].includes(error.code) || optional && error.code === 'EACCES') return '';
    throw error;
  }
}

function kbFields(raw) {
  const result = {};
  for (const [, field, kib] of raw.matchAll(/^(\w+):\s+(\d+) kB$/gm)) result[field] = Number(kib) * 1024;
  return result;
}

function processMemory(pid) {
  const status = kbFields(read('/proc/' + pid + '/status'));
  if (!status.VmRSS) return null;
  const smaps = kbFields(read('/proc/' + pid + '/smaps_rollup', true));
  return { rss: status.VmRSS, hwm: status.VmHWM, rssAnon: status.RssAnon ?? 0,
    rssFile: status.RssFile ?? 0, rssShmem: status.RssShmem ?? 0, pss: smaps.Pss,
    pssAnon: smaps.Pss_Anon, pssFile: smaps.Pss_File, pssShmem: smaps.Pss_Shmem };
}

function cgroupMemory(root) {
  if (!root) return {};
  const raw = read(path.join(root, 'memory.stat'));
  const stats = Object.fromEntries([...raw.matchAll(/^(anon|file|kernel|shmem) (\d+)$/gm)].map(m => [m[1], Number(m[2])]));
  const current = read(path.join(root, 'memory.current')).trim();
  const peak = read(path.join(root, 'memory.peak')).trim();
  assert(current && Number.isFinite(Number(current)) && stats.anon !== undefined && stats.file !== undefined);
  return { cgroupCurrent: Number(current), ...(peak ? { cgroupLifetimePeak: Number(peak) } : {}),
    cgroupAnon: stats.anon, cgroupFile: stats.file, cgroupShmem: stats.shmem, cgroupKernel: stats.kernel };
}

function include(phases, name, now, values) {
  const s = phases[name] ??= { samples: 0, firstMs: now, lastMs: now, peak: {}, min: {}, last: {} };
  s.samples++; s.lastMs = now; s.last = values;
  for (const [field, value] of Object.entries(values)) {
    if (!Number.isFinite(value)) continue;
    s.peak[field] = Math.max(s.peak[field] ?? value, value);
    s.min[field] = Math.min(s.min[field] ?? value, value);
  }
}

export function memorySampler(pid, phasesAt, cgroup = '') {
  const started = performance.now(), phases = {};
  const counters = { missedHelpers: 0, unmeasurableHelperPss: 0, maxGapMs: 0 };
  let error, previous = started, verifiedCgroup = false;
  function sample() {
    try {
      const parent = processMemory(pid);
      if (!parent) return null;
      if (cgroup && !verifiedCgroup) {
        const groupPids = read(path.join(cgroup, 'cgroup.procs')).trim().split(/\s+/).map(Number);
        // Parent is moved by the exec wrapper. Ignore pre-move samples.
        if (!groupPids.includes(pid)) return null;
        verifiedCgroup = true;
      }
      const fields = ['rss','rssAnon','rssFile','rssShmem','pss','pssAnon','pssFile','pssShmem'];
      const helper = Object.fromEntries(fields.map(key => [key, 0]));
      let count = 0, incompletePss = false;
      for (const id of readdirSync('/proc')) {
        if (!/^\d+$/.test(id) || Number(id) === pid) continue;
        const stat = read('/proc/' + id + '/stat');
        if (Number(stat.slice(stat.lastIndexOf(') ') + 2).split(' ')[2]) !== pid) continue;
        const mem = processMemory(id);
        if (!mem) { counters.missedHelpers++; continue; }
        count++;
        if (!Number.isFinite(mem.pss)) { incompletePss = true; counters.unmeasurableHelperPss++; }
        for (const key of fields) helper[key] += mem[key] ?? 0;
      }
      const now = performance.now();
      const values = { ...parent, helpers:count, ...cgroupMemory(cgroup) };
      for (const key of fields) {
        if (key.startsWith('pss') && (incompletePss || parent[key] === undefined)) continue;
        const label = key[0].toUpperCase() + key.slice(1);
        values['helper' + label] = helper[key];
        values['group' + label] = parent[key] + helper[key];
      }
      counters.maxGapMs = Math.max(counters.maxGapMs, now - previous); previous = now;
      for (const name of ['all', ...phasesAt(now, values)]) include(phases, name, now-started, values);

      return values;
    } catch (e) { error ??= e; return null; }
  }
  const timer = setInterval(sample, intervalMs); sample();
  return {
    sample, check() { if (error) throw error; },
    stop() {
      clearInterval(timer); sample();
      if (error) throw error;
      assert(phases.all?.samples && phases.all.peak.pss > 0, 'Missing producer proportional memory samples');
      if (cgroup) assert(verifiedCgroup, 'Producer was not in the requested cgroup');
      return { intervalMs, pid, cgroupMeasured: verifiedCgroup, ...counters, phases };
    },
  };
}
