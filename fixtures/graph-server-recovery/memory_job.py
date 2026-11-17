"""Prepare verified inputs online, then run live/cold process trials in a fresh loopback namespace."""
import hashlib, json, os, pathlib, shutil, subprocess, sys, time, zipfile
from pathlib import Path

SOURCE = '83ca2e2a57e145193e0ee7d2273fa5c70457694d'
BASE = 'd708560ae0e82a6881781876478c40fb4b96f847'
BASE_CARRIER = '5e86ac68c01596ea950d3cb49b2779005ceed79b'
BASE_ELF = '0d9dd6f5be6f8b6f64e4b3145130a83ba9c59034548cf78293d44bf09c2f081a'
BASE_ARCHIVE = 'f5349e0e5d6238af8cb1e1db279eb1e20ef6ea056c1f51f72b83405f33ac3c12'
REPO = 'mmalmi/nostr-social-graph'
HERE = Path(__file__).resolve().parent
STAGE = Path(os.environ['RUNNER_TEMP']) / 'graph-server-alias-repair'
SOURCE_DIR = Path(os.environ['GITHUB_WORKSPACE']) / 'candidate'
GIB = 2**30

def sha(path):
    with open(path,'rb') as file:
        h=hashlib.sha256()
        for part in iter(lambda:file.read(1024**2),b''): h.update(part)
        return h.hexdigest()

def api(path):
    return json.loads(subprocess.check_output(['gh','api',f'repos/{REPO}/{path}'],timeout=60))

def executable(artifact,source,head_name,archive_sha,elf_sha=None):
    assert not artifact['expired'] and artifact['name']=='graph-server-recovery-'+head_name
    assert artifact['size_in_bytes']<150*1024**2 and artifact['digest']=='sha256:'+archive_sha
    file=STAGE/(artifact['name']+'.zip')
    with file.open('xb') as out:
        subprocess.run(['gh','api',f'repos/{REPO}/actions/artifacts/{artifact["id"]}/zip'],stdout=out,timeout=180,check=True)
    assert file.stat().st_size==artifact['size_in_bytes'] and sha(file)==archive_sha
    target=STAGE/('server-'+source)
    with zipfile.ZipFile(file) as z:
        cert=json.loads(z.read('producer-acceptance.json'))
        assert cert['passed'] and cert['sourceUnchanged'] and cert['source']==source and cert['recipeCommit']==head_name
        assert json.loads(z.read('runtime/acceptance.json'))['passed']
        assert not elf_sha or cert['binary']['sha256']==elf_sha
        info=z.getinfo('nostr-social-graph-server')
        assert info.file_size==cert['binary']['bytes'] and 1_000_000<info.file_size<40_000_000
        with target.open('xb') as out,z.open(info) as member: shutil.copyfileobj(member,out)
    assert sha(target)==cert['binary']['sha256']
    target.chmod(0o555)
    return target, cert['binary']['sha256']

def resources():
    mem={k:int(v.split()[0])*1024 for k,v in (row.split(':',1) for row in Path('/proc/meminfo').read_text().splitlines())}
    vm=dict(row.split() for row in Path('/proc/vmstat').read_text().splitlines())
    return dict(available=mem['MemAvailable'],swap=mem['SwapTotal']-mem['SwapFree'],swapout=int(vm['pswpout']),disk=shutil.disk_usage(STAGE).free)

tier,mode=sys.argv[1:];assert tier in ('mobile','full') and mode in ('smoke','paired')
head=os.environ['GITHUB_SHA'];run=os.environ['GITHUB_RUN_ID']
assert subprocess.check_output(['git','rev-parse','HEAD'],text=True,cwd=SOURCE_DIR).strip()==SOURCE
assert subprocess.check_output(['git','rev-parse','HEAD'],text=True,cwd=HERE).strip()==head
assert subprocess.check_output(['node','--version'],text=True).strip()=='v22.23.4'
assert subprocess.check_output(['pnpm','--version'],text=True).strip()=='10.28.2'
subprocess.run(['pnpm','install','--frozen-lockfile','--ignore-scripts'],cwd=SOURCE_DIR,timeout=350,check=True)
base_id=14219929990
baseline,baseline_sha=executable(api('actions/artifacts/'+str(base_id)),BASE,BASE_CARRIER,BASE_ARCHIVE,BASE_ELF)
binaries={'baseline':str(baseline)};hashes={'baseline':baseline_sha};provenance={'baseline':dict(artifactId=base_id,source=BASE,archiveSha256=BASE_ARCHIVE,elf=baseline_sha)}
if mode=='paired':
    artifacts=api('actions/runs/'+run+'/artifacts')['artifacts']
    artifact,=[a for a in artifacts if a['name']=='graph-server-recovery-'+head]
    candidate,candidate_sha=executable(artifact,SOURCE,head,artifact['digest'].removeprefix('sha256:'))
    binaries['candidate']=str(candidate);hashes['candidate']=candidate_sha
    provenance['candidate']=dict(artifactId=artifact['id'],source=SOURCE,archiveSha256=artifact['digest'][7:],elf=candidate_sha)
seed=SOURCE_DIR/'ts/data/socialGraph.bin'
if tier=='full':
    seed=STAGE/'full.bin'
    subprocess.run(['node','--experimental-import-meta-resolve',HERE/'fetch-full.mjs',SOURCE_DIR/'ts/package.json',seed],check=True,timeout=260)
expected={'mobile':(1416341,'d9d11e34916b33fb8540f3a124f5968c0d67db7c45d75412a17c382f011256ca'),'full':(55750905,'aeae583ae888f7fec2052f678ff30ae2b031ed4e8d77d0fe0c644e35c2bd6fc0')}
assert (seed.stat().st_size,sha(seed))==expected[tier]
before=resources();assert before['available']>=(14 if tier=='full' else 5)*GIB and before['disk']>=26*GIB, before
assert sha(STAGE/'htree')=='67306642b1c581d73e89f2d533724f24b877a2b9d5bd59557d522b6f93232850'
assert sha(STAGE/'legacy-htree')=='f3471a3f29d5169cb4e0929cd5554dad9f1b86dabed5b9c3ea3b51b0d071923e'
config=dict(source=str(SOURCE_DIR),stage=str(STAGE),evidence=str(STAGE/'evidence'),path=os.environ['PATH'],uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),hostPid=os.readlink('/proc/self/ns/pid'),binaries=binaries,hashes=hashes,inputs=provenance,node=shutil.which('node'),seed=str(seed),tier=tier,mode=mode,runtimeName='memory')
cfg=STAGE/'memory-input.json';cfg.write_text(json.dumps(config))
cmd=['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',sys.executable,HERE/'memory_namespace.py',cfg]
p=subprocess.Popen(cmd);error=None;floor=before.copy();started=time.monotonic()
try:
    while p.poll() is None:
        now=resources()
        for key in ('available','disk'):floor[key]=min(floor[key],now[key])
        assert now['available']>=2*GIB and now['disk']>=21*GIB and before['disk']-now['disk']<6*GIB, now
        assert now['swap']<=before['swap'] and now['swapout']==before['swapout'], now
        assert time.monotonic()-started<((2 if mode=='paired' else 1)*(1830 if tier=='full' else 930)+30)
        time.sleep(.5)
    assert p.returncode==0, p.returncode
except BaseException as exc:
    error=repr(exc)
    if p.poll() is None:
        subprocess.run(['sudo','-n','kill','-TERM',str(p.pid)],check=False)
        try:p.wait(timeout=8)
        except subprocess.TimeoutExpired:subprocess.run(['sudo','-n','kill','-KILL',str(p.pid)],check=False);p.wait(timeout=5)
finally:
    (STAGE/'evidence'/'memory-host.json').write_text(json.dumps(dict(source=SOURCE,recipe=head,tier=tier,mode=mode,inputs=provenance,before=before,floor=floor,after=resources(),error=error,passed=not error),indent=2)+'\n')
if error:raise RuntimeError(error)
