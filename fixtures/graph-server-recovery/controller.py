"""Task-only Linux producer. No deployment, signing, source push, or cache cleanup."""
import hashlib, json, os, pathlib, re, shutil, signal, subprocess, sys, tarfile, time, urllib.request

SOURCE = '75b1562d2aa8a028052c4dab7e766a192e8bc8bd'
NODE = '22.23.4'
NODE_SHA = '1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce'
NODE_URL = f'https://nodejs.org/dist/v{NODE}/node-v{NODE}-linux-x64.tar.xz'
GIB = 1024 ** 3
HERE = pathlib.Path(__file__).resolve().parent
STAGE = pathlib.Path(os.environ['RUNNER_TEMP']) / 'graph-server-recovery-75b156'
EVIDENCE = STAGE / 'evidence'
CANDIDATE = pathlib.Path(os.environ['GITHUB_WORKSPACE']) / 'candidate'
START = time.monotonic()
BASELINE = None
OWNED = {}
PHASES = []
LIMITS = dict(totalSeconds=2700, entryDiskBytes=28*GIB, runtimeDiskBytes=21*GIB,
              growthBytes=6*GIB, entryAvailableBytes=5*GIB, runtimeAvailableBytes=GIB,
              ownRssBytes=4*GIB, jobs=1, noNewSwapouts=True, noNewSwapUsage=True)

def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1024*1024), b''): h.update(block)
    return h.hexdigest()

def write(path, value):
    pathlib.Path(path).write_text(json.dumps(value, indent=2)+'\n')

def sample():
    mem = dict((a.rstrip(':'), int(b)*1024) for a,b,*_ in
               (line.split() for line in pathlib.Path('/proc/meminfo').read_text().splitlines()))
    vm = dict(line.split() for line in pathlib.Path('/proc/vmstat').read_text().splitlines())
    return dict(at=time.time(), available=mem['MemAvailable'],
                swapUsed=mem['SwapTotal']-mem['SwapFree'], swapouts=int(vm['pswpout']),
                diskFree=shutil.disk_usage(STAGE).free)

def rows():
    result = {}
    for p in pathlib.Path('/proc').iterdir():
        if not p.name.isdigit(): continue
        try:
            fields = (p/'stat').read_text().rsplit(')',1)[1].split()
            result[int(p.name)] = dict(ppid=int(fields[1]), pgid=int(fields[2]),
                                      start=fields[19], rss=int(fields[21])*os.sysconf('SC_PAGE_SIZE'),
                                      zombie=fields[0]=='Z')
        except (FileNotFoundError, ProcessLookupError, PermissionError): pass
    return result

def descendants(pid):
    snapshot = rows(); members = {pid}; changed = True
    while changed:
        old = len(members)
        members.update(p for p,r in snapshot.items() if r['ppid'] in members or r['pgid']==pid)
        changed = len(members)!=old
    for p in members:
        if p in snapshot: OWNED[p]=snapshot[p]['start']
    return [(p,r) for p,r in snapshot.items() if OWNED.get(p)==r['start']]

def reap(child):
    # Track start times so recycled PIDs can never enter task-owned cleanup.
    for sig in (signal.SIGTERM, signal.SIGKILL):
        remaining = descendants(child.pid)
        for pid, row in reversed(remaining):
            if row['zombie']: continue
            try: os.kill(pid,sig)
            except ProcessLookupError: pass
            except PermissionError:
                subprocess.run(['sudo','-n','kill',f'-{sig.value}',str(pid)], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3)
        try: child.wait(timeout=5)
        except subprocess.TimeoutExpired: pass
        until = time.monotonic()+5
        while time.monotonic()<until and any(not r['zombie'] for _,r in descendants(child.pid)):
            time.sleep(.1)
    remaining = [(p,r) for p,r in descendants(child.pid) if not r['zombie']]
    assert not remaining, f'Task descendants remain: {[p for p,_ in remaining]}'

def guard(current):
    assert time.monotonic()-START <= LIMITS['totalSeconds'], 'Absolute qualification deadline'
    assert current['available'] >= LIMITS['runtimeAvailableBytes'], 'Available-memory reserve'
    assert current['diskFree'] >= LIMITS['runtimeDiskBytes'], '20GiB floor plus1GiB response margin'
    assert BASELINE['diskFree']-current['diskFree'] <= LIMITS['growthBytes'], '6GiB growth limit'
    assert current['swapouts']==BASELINE['swapouts'], 'New host swapouts'
    assert current['swapUsed']<=BASELINE['swapUsed'], 'New host swap usage'

def run(name, argv, seconds, cwd=None, env=None):
    log = EVIDENCE/(name+'.log'); assert not log.exists()
    record = dict(name=name, argv=list(map(str,argv)), seconds=seconds, start=time.time(), before=sample())
    guard(record['before']); child=None; error=None
    minimum=record['before'].copy(); peak=0; code=None
    try:
        with log.open('xb') as out:
            child=subprocess.Popen(list(map(str,argv)),cwd=cwd,env=env,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
            begun=time.monotonic()
            while child.poll() is None:
                now=sample(); members=descendants(child.pid); rss=sum(r['rss'] for _,r in members)
                peak=max(peak,rss); minimum['available']=min(minimum['available'],now['available']); minimum['diskFree']=min(minimum['diskFree'],now['diskFree'])
                guard(now); assert rss<=LIMITS['ownRssBytes'],'Own process RSS limit'
                assert time.monotonic()-begun<=seconds,f'{name} deadline'
                time.sleep(.5)
            code=child.wait(); assert code==0,f'{name} exited {code}'
    except BaseException as exc: error=repr(exc)
    finally:
        if child:
            try: reap(child)
            except BaseException as exc: error=error or repr(exc)
        record.update(exitCode=code,error=error,after=sample(),minimum=minimum,peakOwnRss=peak,
                      elapsed=time.time()-record['start'],processesReaped=not any(not r['zombie'] for p,r in rows().items() if OWNED.get(p)==r['start']))
        try: guard(record['after'])
        except BaseException as exc: record['error']=record['error'] or repr(exc)
        record['passed']=record['error'] is None
        record['logSha256']=sha(log); write(EVIDENCE/(name+'.json'),record); PHASES.append(record)
        print(json.dumps({k:record[k] for k in ['name','passed','exitCode','error','elapsed','peakOwnRss','processesReaped']}),flush=True)
    assert record['passed'],record['error']
    return log

def out(argv, **kwargs):
    return subprocess.check_output(list(map(str,argv)),text=True,timeout=30,**kwargs).strip()

def interrupted(signum, _frame):
    raise RuntimeError(f'Qualification interrupted by signal{signum}')

for signum in (signal.SIGTERM, signal.SIGINT): signal.signal(signum,interrupted)

mode=sys.argv[1]; assert mode in ('bootstrap','qualify')
if mode=='bootstrap':
    STAGE.mkdir(); EVIDENCE.mkdir(); BASELINE=sample()
    assert BASELINE['diskFree']>=LIMITS['entryDiskBytes'] and BASELINE['available']>=LIMITS['entryAvailableBytes']
    write(EVIDENCE/'admission.json',dict(source=SOURCE,node=dict(version=NODE,url=NODE_URL,sha256=NODE_SHA),before=BASELINE,limits=LIMITS))
    # curl has its own finite network/retry limits and runs under the same guard.
    archive=STAGE/'node.tar.xz'
    run('node-download',['curl','--fail','--location','--max-time','120','--retry','2','--output',archive,NODE_URL],180)
    assert sha(archive)==NODE_SHA,'Official Node artifact hash differs'
    run('node-extract',['tar','-xJf',archive,'-C',STAGE],60)
    nodebin=STAGE/f'node-v{NODE}-linux-x64/bin'
    assert out([nodebin/'node','--version'])=='v'+NODE
    with open(os.environ['GITHUB_PATH'],'a') as f:f.write(str(nodebin)+'\n')
    sys.exit(0)

admission=json.loads((EVIDENCE/'admission.json').read_text()); BASELINE=admission['before']
assert out(['git','rev-parse','HEAD'],cwd=CANDIDATE)==SOURCE
assert not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE)
assert out(['node','--version'])=='v'+NODE and out(['pnpm','--version'])=='10.28.2'
entry=sample();guard(entry);assert entry['available']>=LIMITS['entryAvailableBytes'] and entry['diskFree']>=LIMITS['entryDiskBytes']
result=dict(source=SOURCE,recipeCommit=out(['git','rev-parse','HEAD'],cwd=HERE),limits=LIMITS,passed=False)
try:
    env=dict(os.environ,CARGO_TARGET_DIR=str(STAGE/'target'),CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',
             CARGO_PROFILE_RELEASE_DEBUG='0',RUSTFLAGS='',CARGO_ENCODED_RUSTFLAGS='',RUSTC_WRAPPER='',RAYON_NUM_THREADS='1')
    run('rust-toolchain',['rustup','toolchain','install','1.99.0','--profile','minimal'],300,env=env)
    cargo=out(['rustup','which','--toolchain','1.99.0','cargo']);rustc=out(['rustup','which','--toolchain','1.99.0','rustc'])
    env['RUSTC']=rustc
    assert 'host: x86_64-unknown-linux-gnu' in out([rustc,'-Vv'])
    run('source-history',['git','fsck','--full','--strict'],120,cwd=CANDIDATE,env=env)
    run('pnpm-frozen',['pnpm','install','--frozen-lockfile','--ignore-scripts'],300,cwd=CANDIDATE,env=env)
    preflight=dict(source=str(CANDIDATE),stage=str(STAGE),evidence=str(EVIDENCE),node=shutil.which('node'),path=env['PATH'],
                   uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),hostPid=os.readlink('/proc/self/ns/pid'),
                   binary='',harnesses=[],boundaryOnly=True,runtimeName='boundary-preflight')
    write(EVIDENCE/'boundary-input.json',preflight)
    run('isolated-preflight',['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',
                             sys.executable,HERE/'namespace.py',EVIDENCE/'boundary-input.json'],30,env=env)
    run('cargo-prefetch',[cargo,'fetch','--locked','--target','x86_64-unknown-linux-gnu'],300,cwd=CANDIDATE/'rust',env=env)
    metadata=json.loads(out([cargo,'metadata','--frozen','--filter-platform','x86_64-unknown-linux-gnu','--format-version','1'],cwd=CANDIDATE/'rust',env=env))
    write(EVIDENCE/'cargo-metadata.json',metadata)
    build=run('release-test-build',[cargo,'test','--release','--frozen','--workspace','--no-run','--message-format=json'],1500,cwd=CANDIDATE/'rust',env=env)
    harnesses=[]
    for line in build.read_text().splitlines():
        try: event=json.loads(line)
        except json.JSONDecodeError: continue
        if event.get('reason')=='compiler-artifact' and event.get('profile',{}).get('test') and event.get('executable'):
            harnesses.append(dict(path=event['executable'],package=event['package_id'],target=event['target'],cwd=str(pathlib.Path(event['manifest_path']).parent),sha256=sha(event['executable'])))
    assert harnesses and any(h['target']['name']=='nostr_social_graph' for h in harnesses)
    assert len({h['path'] for h in harnesses})==len(harnesses)
    # Preserve bounded executable artifacts, not a Cargo cache, for runtime-only
    # qualification retries with unchanged source/toolchain/compiled bytes.
    harness_dir=EVIDENCE/'compiled-harnesses';harness_dir.mkdir()
    assert sum(pathlib.Path(h['path']).stat().st_size for h in harnesses)<=512*1024**2
    for harness in harnesses:
        archive=harness_dir/pathlib.Path(harness['path']).name
        shutil.copy2(harness['path'],archive)
        assert sha(archive)==harness['sha256']
        harness['artifact']='compiled-harnesses/'+archive.name
    write(EVIDENCE/'test-harnesses.json',harnesses)
    run('release-server-build',[cargo,'build','--release','--frozen','-p','nostr-social-graph-server','--bin','nostr-social-graph-server'],1200,cwd=CANDIDATE/'rust',env=env)
    binary=STAGE/'target/release/nostr-social-graph-server'; artifact=EVIDENCE/'nostr-social-graph-server'
    shutil.copy2(binary,artifact); artifact.chmod(0o755)
    libs=out(['ldd',binary]); (EVIDENCE/'ldd.txt').write_text(libs+'\n')
    libraryPaths=set(re.findall(r'(?:=>\s+)?(/[^\s()]+)',libs)); libraries={p:dict(sha256=sha(p),bytes=pathlib.Path(p).stat().st_size) for p in sorted(libraryPaths)}
    versions=out(['readelf','--version-info',binary]);(EVIDENCE/'elf-versions.txt').write_text(versions+'\n')
    glibc=max((tuple(map(int,x.split('.'))) for x in re.findall(r'GLIBC_([0-9.]+)',versions)))
    assert glibc<=(2,36),'Binary exceeds established Debian12 glibc'
    with open(EVIDENCE/'candidate-source.tar','wb') as f:subprocess.run(['git','archive','--format=tar',SOURCE],cwd=CANDIDATE,stdout=f,check=True,timeout=30)
    locks={p:sha(CANDIDATE/p) for p in ['rust/Cargo.lock','pnpm-lock.yaml','package.json','ts/package.json']}
    result.update(binary=dict(sha256=sha(artifact),bytes=artifact.stat().st_size,file=out(['file',artifact]),glibcMaximum='.'.join(map(str,glibc))),
                  toolchain=dict(target='x86_64-unknown-linux-gnu',profile='release',rustc=out([rustc,'-Vv']),rustcSha256=sha(rustc),cargo=out([cargo,'--version']),cargoSha256=sha(cargo),rustup=out(['rustup','--version']),node=out(['node','--version']),nodeSha256=sha(shutil.which('node')),pnpm=out(['pnpm','--version'])),
                  system=dict(runnerImageOS=os.environ.get('ImageOS'),runnerImageVersion=os.environ.get('ImageVersion'),osRelease=pathlib.Path('/etc/os-release').read_text(),kernel=out(['uname','-a']),packages=out(['dpkg-query','-W','libc6','libssl3','libgcc-s1','libstdc++6']),libraries=libraries),
                  locks=locks,sourceArchiveSha256=sha(EVIDENCE/'candidate-source.tar'),harnesses=len(harnesses),
                  oldNegative='Exact old128-author/no-limit wire filter gets100/256+EOSE; frozen legacy prune oracle. No old-binary match claimed.')
    shutil.copytree(HERE,EVIDENCE/'recipe')
    shutil.copy2(HERE.parents[1]/'.github/workflows/graph-server-recovery.yml',EVIDENCE/'recipe-workflow.yml')
    runtime=dict(source=str(CANDIDATE),stage=str(STAGE),evidence=str(EVIDENCE),node=shutil.which('node'),path=env['PATH'],
                 uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),hostPid=os.readlink('/proc/self/ns/pid'),binary=str(binary),harnesses=harnesses)
    write(EVIDENCE/'runtime-input.json',runtime)
    run('isolated-tests',['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',
                          sys.executable,HERE/'namespace.py',EVIDENCE/'runtime-input.json'],900,env=env)
    assert sha(artifact)==sha(binary),'Candidate binary mutated during tests'
    assert not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE),'Candidate tracked source changed'
    assert all(sha(CANDIDATE/p)==digest for p,digest in locks.items())
    result.update(passed=True,sourceUnchanged=True,registryPublication=False,deployed=False)
except BaseException as exc:
    result['error']=repr(exc)
finally:
    result.update(phases=[dict(name=p['name'],passed=p['passed'],receipt=p['name']+'.json') for p in PHASES],after=sample(),elapsedSeconds=time.monotonic()-START)
    write(EVIDENCE/'producer-acceptance.json',result)
    print(json.dumps({k:result[k] for k in ['source','passed','elapsedSeconds']}),flush=True)
assert result['passed'],result.get('error')
