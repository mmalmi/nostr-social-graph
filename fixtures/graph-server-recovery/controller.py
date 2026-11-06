"""Task-only Linux producer. No deployment, signing, source push, or cache cleanup."""
import hashlib, json, os, pathlib, re, shutil, signal, subprocess, sys, tarfile, time, urllib.request

SOURCE = 'adf438eebc0e68c8f982e761acdef65e498ed92a'
NODE = '22.23.4'
NODE_SHA = '1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce'
NODE_URL = f'https://nodejs.org/dist/v{NODE}/node-v{NODE}-linux-x64.tar.xz'
GIB = 1024 ** 3
HERE = pathlib.Path(__file__).resolve().parent
STAGE = pathlib.Path(os.environ['RUNNER_TEMP']) / 'graph-server-recovery-replay'
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
COMPILED_SOURCE='75b1562d2aa8a028052c4dab7e766a192e8bc8bd'
ARTIFACT_SHA='6697d723deeb0168da83636561016c83051bf7afb0d4e53df2a941675ae636ee'
BINARY_SHA='c04ec11442c269b8d0f1531d0bec0e01b8709acedc671d48ed67cd61b97ca5cd'
assert out(['git','rev-parse','HEAD'],cwd=CANDIDATE)==SOURCE
assert not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE)
assert out(['git','diff','--name-only',COMPILED_SOURCE,SOURCE],cwd=CANDIDATE)=='rust/crates/nostr-social-graph-server/tests/crawl_cap.mjs'
assert sha(CANDIDATE/'rust/crates/nostr-social-graph-server/tests/crawl_cap.mjs')=='357b2ff38c7ec1a1f665973bd87f2ea47c52e0e2ca7c822489d60ac818e340be'
assert out(['node','--version'])=='v'+NODE and out(['pnpm','--version'])=='10.28.2'
entry=sample();guard(entry);assert entry['available']>=LIMITS['entryAvailableBytes'] and entry['diskFree']>=LIMITS['entryDiskBytes']
result=dict(source=SOURCE,compiledSource=COMPILED_SOURCE,recipeCommit=out(['git','rev-parse','HEAD'],cwd=HERE),passed=False,compilationPerformed=False)
try:
    import zipfile, urllib.error, urllib.parse
    # Do not forward the GitHub token to the signed artifact storage redirect.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self,*args,**kwargs):return None
    request=urllib.request.Request('https://api.github.com/repos/mmalmi/nostr-social-graph/actions/artifacts/13468517245/zip',headers={'Authorization':'Bearer '+os.environ['GH_TOKEN'],'Accept':'application/vnd.github+json'})
    try:urllib.request.build_opener(NoRedirect).open(request,timeout=30);raise AssertionError('Expected artifact redirect')
    except urllib.error.HTTPError as exc:
        assert exc.code==302;url=exc.headers['Location'];assert urllib.parse.urlparse(url).scheme=='https'
    archive=STAGE/'compiled-artifact.zip'
    with urllib.request.urlopen(url,timeout=60) as response,archive.open('xb') as target:
        total=0
        while block:=response.read(1024**2):
            total+=len(block);assert total<=40*1024**2;guard(sample());target.write(block)
    assert total==38393007 and sha(archive)==ARTIFACT_SHA
    retained=STAGE/'retained';retained.mkdir()
    with zipfile.ZipFile(archive) as packed:
        assert sum(m.file_size for m in packed.infolist())<512*1024**2
        for member in packed.infolist():
            path=pathlib.PurePosixPath(member.filename)
            assert not path.is_absolute() and '..' not in path.parts
            assert (member.external_attr>>16)&0o170000 != 0o120000
        packed.extractall(retained)
    prior=json.loads((retained/'producer-acceptance.json').read_text())
    assert prior['source']==COMPILED_SOURCE and prior['binary']['sha256']==BINARY_SHA
    assert [p['name'] for p in prior['phases'] if not p['passed']]==['isolated-tests']
    assert sha(retained/'candidate-source.tar')==prior['sourceArchiveSha256']
    units=sorted((retained/'runtime').glob('unit-*.json'));assert len(units)==27
    counts=dict(passed=0,failed=0,ignored=0)
    for receipt in units:
        item=json.loads(receipt.read_text());assert item['passed'] and not item['remainingProcesses']
        log=receipt.with_suffix('.log');assert sha(log)==item['logSha256']
        matches=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',log.read_text());assert len(matches)==1
        for key,value in zip(counts,matches[0]):counts[key]+=int(value)
    assert counts==dict(passed=286,failed=0,ignored=1)
    bench=json.loads((retained/'runtime/release-prune-benchmark.json').read_text());assert bench['passed'] and not bench['remainingProcesses']
    assert sha(retained/'runtime/release-prune-benchmark.log')==bench['logSha256']
    harnesses=json.loads((retained/'test-harnesses.json').read_text());assert len(harnesses)==27
    for harness in harnesses:assert sha(retained/harness['artifact'])==harness['sha256']
    binary=retained/'nostr-social-graph-server';assert sha(binary)==BINARY_SHA;binary.chmod(0o755)
    locks={p:sha(CANDIDATE/p) for p in prior['locks']};assert locks==prior['locks']
    run('source-history',['git','fsck','--full','--strict'],120,cwd=CANDIDATE)
    run('pnpm-frozen',['pnpm','install','--frozen-lockfile','--ignore-scripts'],300,cwd=CANDIDATE)
    libs=out(['ldd',binary]);assert 'not found' not in libs;(EVIDENCE/'ldd.txt').write_text(libs+'\n')
    libraries={p:dict(sha256=sha(p),bytes=pathlib.Path(p).stat().st_size) for p in sorted(set(re.findall(r'(?:=>\s+)?(/[^\s()]+)',libs)))}
    runtime=dict(source=str(CANDIDATE),stage=str(STAGE),evidence=str(EVIDENCE),node=shutil.which('node'),path=os.environ['PATH'],uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),hostPid=os.readlink('/proc/self/ns/pid'),binary=str(binary),harnesses=harnesses,runtimeOnly=True,reusedUnitCounts=counts)
    write(EVIDENCE/'runtime-input.json',runtime)
    run('isolated-recovery-replay',['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',sys.executable,HERE/'namespace.py',EVIDENCE/'runtime-input.json'],900)
    assert sha(binary)==BINARY_SHA and not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE)
    assert all(sha(CANDIDATE/p)==digest for p,digest in locks.items())
    shutil.copy2(binary,EVIDENCE/'nostr-social-graph-server')
    shutil.copy2(retained/'producer-acceptance.json',EVIDENCE/'original-producer.json')
    shutil.copy2(retained/'runtime/release-prune-benchmark.log',EVIDENCE/'reused-prune-benchmark.log')
    result.update(passed=True,binary=prior['binary'],originalArtifact=dict(id=13468517245,run=42141875595,sha256=ARTIFACT_SHA,bytes=total),reusedUnits=counts,reusedHarnessHashesVerified=27,reusedBenchmarkPassed=True,toolchain=prior['toolchain'],originalSystem=prior['system'],runtimeSystem=dict(osRelease=pathlib.Path('/etc/os-release').read_text(),libraries=libraries),locks=locks,sourceUnchanged=True,deployed=False,registryPublished=False)
except BaseException as exc:result['error']=repr(exc)
finally:
    result.update(phases=[dict(name=p['name'],passed=p['passed'],receipt=p['name']+'.json') for p in PHASES],after=sample(),elapsedSeconds=time.monotonic()-START)
    write(EVIDENCE/'replay-acceptance.json',result)
    print(json.dumps({k:result[k] for k in ['source','passed','elapsedSeconds']}),flush=True)
assert result['passed'],result.get('error')
