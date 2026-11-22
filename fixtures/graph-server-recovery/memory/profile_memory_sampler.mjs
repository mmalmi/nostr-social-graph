// Verify helper process identity on both sides of each non-atomic /proc read.
// Dedicated cgroup counters remain authoritative across helper fork and exec.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
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
  if (values.groupPss !== undefined) s.completeProcessSamples = (s.completeProcessSamples ?? 0) + 1;
  else s.withheldProcessSamples = (s.withheldProcessSamples ?? 0) + 1;
  for (const [field, value] of Object.entries(values)) {
    if (!Number.isFinite(value)) continue;
    s.peak[field] = Math.max(s.peak[field] ?? value, value);
    s.min[field] = Math.min(s.min[field] ?? value, value);
  }
}

const procFields = ['rss','rssAnon','rssFile','rssShmem','pss','pssAnon','pssFile','pssShmem'];
const sum = () => Object.fromEntries(procFields.map(k => [k,0]));
const title = key => key[0].toUpperCase() + key.slice(1);

function executable(file) {
  try {
    const {dev, ino} = statSync(file,{bigint:true});
    return dev.toString()+':'+ino.toString();
  } catch (error) {
    if (['ENOENT','ESRCH','EACCES','EPERM'].includes(error.code)) return null;
    throw error;
  }
}

function identity(id) {
  const raw = read('/proc/'+id+'/stat');
  if (!raw) return null;
  // After the parenthesized command, offset zero is stat field 3.
  const end = raw.lastIndexOf(') '), f = raw.slice(end+2).trim().split(' ');
  if (end < 0 || f.length < 48 || ['Z','X','x'].includes(f[0])) return null;
  const exe=executable('/proc/'+id+'/exe');
  const layout=[f[23],f[24],f[25],f[42],f[43],f[44]].join(':');
  return {id, parent:Number(f[1]), group:Number(f[2]), session:Number(f[3]), start:f[19], exe, layout};
}

function sameIdentity(a,b) {
  return Boolean(a && b && a.id===b.id && a.parent===b.parent && a.group===b.group &&
    a.session===b.session && a.start===b.start && a.exe && a.exe===b.exe && a.layout===b.layout);
}

function members(pid, cgroup) {
  if(cgroup) {
    return new Set(read(path.join(cgroup,'cgroup.procs')).trim().split(/\s+/).filter(Boolean).map(Number));
  }
  const ids = new Set();
  for (const item of readdirSync('/proc')) {
    if (!/^\d+$/.test(item)) continue;
    const id=Number(item), proc=identity(id);
    if (proc?.group===pid && proc.session===pid) ids.add(id);
  }
  return ids;
}

// Keep reasons separate so a transient duplicate address space never becomes a
// helper maximum. If any member is uncertain, omit the whole /proc sum for that
// sample while retaining cgroup counters and individually verified measurements.
function sampleProcesses(pid, cgroup, ids, images, diagnostics) {
  const starts = new Map([...ids].map(id=>[id,identity(id)]));
  const memory = new Map([...ids].map(id=>[id,processMemory(id)]));
  const ends = new Map([...ids].map(id=>[id,identity(id)]));
  const finalIds=members(pid,cgroup);
  const reasons=new Set();
  const suspect=(reason)=>{diagnostics.observations[reason]=(diagnostics.observations[reason]??0)+1;reasons.add(reason);};
  if ([...ids].some(id=>!finalIds.has(id)) || [...finalIds].some(id=>!ids.has(id))) suspect('membershipChanged');
  const main=memory.get(pid);
  const mainStart=starts.get(pid), mainEnd=ends.get(pid);
  const mainOK = sameIdentity(mainStart,mainEnd) && mainStart.exe===images.producer && mainStart.group===pid &&
    mainStart.session===pid && finalIds.has(pid) && main && procFields.every(k=>Number.isFinite(main[k]));
  if(!mainOK) suspect('producerTransition');
  const helper=sum(), results={};
  const counts = {htree:0,df:0};
  for (const id of ids) {
    if (id===pid) continue;
    const a=starts.get(id),b=ends.get(id),m=memory.get(id);
    if (!a || !b || !m || !finalIds.has(id)) {suspect('exited');continue;}
    if (!a.exe || !b.exe) {suspect('unreadableExe');continue;}
    if (!sameIdentity(a,b) || a.group!==pid || a.session!==pid) {suspect('changedIdentity');continue;}
    if (a.exe===images.producer) {suspect('producerImage');continue;}
    const type = a.exe===images.htree?'htree':a.exe===images.df?'df':null;
    if (!type) {suspect('otherImage');continue;}
    if (!procFields.every(k=>Number.isFinite(m[k]))) {suspect('incompleteMemory');continue;}
    // A helper can itself fork. Same executable AND unchanged address layout
    // as its in-scope parent is ambiguous until the child has installed its image.
    const parent = starts.get(a.parent);
    if (parent?.exe===a.exe && parent.layout===a.layout) {suspect('inheritedHelperImage');continue;}
    counts[type]++;
    diagnostics.stableProcessObservations[type]++;
    for(const key of procFields) {
      helper[key]+=m[key];
      const metric=type+'Single'+title(key);
      results[metric]=Math.max(results[metric]??0,m[key]);
    }
  }
  const visible=new Set([...ids,...finalIds]);visible.delete(pid);
  const values={...(mainOK ? main : {}),...results,helpers:visible.size,stableHtree:counts.htree,stableDf:counts.df};
  if (reasons.size) {
    diagnostics.withheldSamples++;
    for (const reason of reasons) diagnostics.samplesByReason[reason]=(diagnostics.samplesByReason[reason]??0)+1;
  } else {
    diagnostics.completeSamples++;
    for(const key of procFields) {
      values['helper'+title(key)]=helper[key];values['group'+title(key)]=main[key]+helper[key];
    }
  }
  return values;
}

export function memorySampler(pid, phasesAt, cgroup='', paths) {
  const images={producer:executable(paths.producer),htree:executable(paths.htree),df:executable('/usr/bin/df')};
  assert(Object.values(images).every(Boolean) && new Set(Object.values(images)).size===3);
  const started=performance.now(),phases={};
  const helperIdentity={completeSamples:0,withheldSamples:0,samplesByReason:{},observations:{},stableProcessObservations:{htree:0,df:0}};
  let error,previous=started,verifiedCgroup=false,maxGapMs=0;
  function sample() {
    try {
      const ids=members(pid,cgroup);
      if (!ids.has(pid)) return null;
      if(cgroup)verifiedCgroup=true;
      const values={...sampleProcesses(pid,cgroup,ids,images,helperIdentity),...cgroupMemory(cgroup)};
      const now=performance.now();
      maxGapMs=Math.max(maxGapMs,now-previous);previous=now;
      for (const name of ['all',...phasesAt(now,values)]) include(phases,name,now-started,values);
      return values;
    } catch(e){error??=e;return null;}
  }
  const timer=setInterval(sample,intervalMs);sample();
  return {
    sample,check(){if(error)throw error;},
    stop(){
      clearInterval(timer);sample();if(error)throw error;
      assert(phases.all?.completeProcessSamples && phases.all.peak.pss>0,'Missing stable producer samples');
      if(cgroup)assert(verifiedCgroup && phases.all.peak.cgroupLifetimePeak>0);
      return {intervalMs,pid,cgroupMeasured:verifiedCgroup,maxGapMs,helperIdentity,phases,
        missedHelpers:helperIdentity.observations.exited??0,unmeasurableHelperPss:helperIdentity.observations.incompleteMemory??0};
    },
  };
}
