"""Task-only Linux producer. No deployment, signing, source push, or cache cleanup."""
import hashlib, json, os, pathlib, re, shutil, signal, subprocess, sys, tarfile, time, urllib.request

SOURCE = 'c7bfd23132e8dc1c024cdc39e0b5ca00c7746c67'
NODE = '22.23.4'
NODE_SHA = '1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce'
NODE_URL = f'https://nodejs.org/dist/v{NODE}/node-v{NODE}-linux-x64.tar.xz'
HTREE_SOURCE = '6058c7b3e3807d11b828459fa1d39c2e702e69bd'
HTREE_VERSION = '0.4.5'
HTREE_ASSET = 'hashtree-x86_64-unknown-linux-musl.tar.gz'
HTREE_ARCHIVE_BYTES = 53_117_385
HTREE_ARCHIVE_SHA = '405ffbf9ac145b7548f0ae5767d689d362b9923efdd6690613f9c383be535bf8'
HTREE_BINARY_BYTES = 87_319_104
HTREE_BINARY_SHA = '67306642b1c581d73e89f2d533724f24b877a2b9d5bd59557d522b6f93232850'
LEGACY_HTREE_VERSION = '0.2.77'
LEGACY_HTREE_SOURCE = '2c1a6eb1d417a83fb30b44a1c154015e6e21957e'
LEGACY_HTREE_ARCHIVE_SHA = 'a196d1154c2e11e6cb1b0c07162dd0c37ed262a3e42a53085af9b13e9c398c51'
LEGACY_HTREE_BINARY_SHA = 'f3471a3f29d5169cb4e0929cd5554dad9f1b86dabed5b9c3ea3b51b0d071923e'
LEGACY_HTREE_RELEASE_SHA = '1f2b0b8d0dcb62ff95e3bf13e4f665c957ec157b8f28749154fea9e6ccf5aabf'
NOTIFIER_SOURCE = '32fb561c24bf1ebe078edf4781deabc21e34e1c0'
NOTIFIER_PARSER_SHA = '0ac11030f643a747f8436437984da0cdd5e5606b08d975dda900bde11d740228'
GIB = 1024 ** 3
HERE = pathlib.Path(__file__).resolve().parent
STAGE = pathlib.Path(os.environ['RUNNER_TEMP']) / 'graph-server-alias-repair'
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

mode=sys.argv[1]; assert mode in ('bootstrap','qualify','transport-replay')
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
    for name, version, commit, size, archive_sha, binary_size, binary_sha in [
        ('htree', HTREE_VERSION, HTREE_SOURCE, HTREE_ARCHIVE_BYTES, HTREE_ARCHIVE_SHA, HTREE_BINARY_BYTES, HTREE_BINARY_SHA),
        ('legacy-htree', LEGACY_HTREE_VERSION, LEGACY_HTREE_SOURCE, 40_465_907, LEGACY_HTREE_ARCHIVE_SHA, 53_403_712, LEGACY_HTREE_BINARY_SHA),
    ]:
        release_url = f'https://github.com/mmalmi/hashtree/releases/download/v{version}'
        url = release_url + '/' + HTREE_ASSET
        archive_path = STAGE / (name + '.tar.gz')
        run(name + '-download', ['curl', '--fail', '--location', '--max-time', '120',
            '--retry', '2', '--output', archive_path, url], 180)
        assert archive_path.stat().st_size == size and sha(archive_path) == archive_sha
        if name == 'htree':
            run('htree-provenance', ['gh', 'attestation', 'verify', archive_path,
                '--repo', 'mmalmi/hashtree',
                '--signer-workflow', 'mmalmi/hashtree/.github/workflows/release.yml',
                '--source-digest', commit, '--deny-self-hosted-runners'], 90)
            provenance = {'attestationReceipt': 'htree-provenance.json'}
        else:
            # v0.2.77 predates upstream attestations. Pin the upstream release
            # manifest and both artifact hashes for this compatibility reader.
            metadata = EVIDENCE / 'legacy-htree-release.json'
            run('legacy-htree-manifest', ['curl', '--fail', '--location', '--max-time',
                '30', '--output', metadata, release_url + '/release.json'], 45)
            assert sha(metadata) == LEGACY_HTREE_RELEASE_SHA
            release = json.loads(metadata.read_text())
            assert release['tag'] == f'v{version}' and release['commit'] == commit
            assert any(a['name'] == HTREE_ASSET and a['size'] == size for a in release['assets'])
            provenance = {'releaseManifestSha256': LEGACY_HTREE_RELEASE_SHA}
        executable = STAGE / name
        with tarfile.open(archive_path, 'r:gz') as archive:
            member = archive.getmember('hashtree/htree')
            assert member.isfile() and member.size == binary_size
            with archive.extractfile(member) as source, executable.open('xb') as dest:
                shutil.copyfileobj(source, dest)
        assert sha(executable) == binary_sha
        executable.chmod(0o755)
        write(EVIDENCE / (name + '-artifact.json'), dict(source=commit, version=version,
            url=url, archiveBytes=size, archiveSha256=archive_sha,
            binaryBytes=binary_size, binarySha256=binary_sha, **provenance))
    probe_source = STAGE / 'notifier-probe-source'
    probe_source.mkdir()
    parser = probe_source / 'external_social_graph.rs'
    parser_url = f'https://raw.githubusercontent.com/mmalmi/nostr-notification-server/{NOTIFIER_SOURCE}/src/external_social_graph.rs'
    run('notifier-parser-download', ['curl', '--fail', '--location', '--max-time', '60',
        '--retry', '1', '--output', parser, parser_url], 90)
    assert parser.stat().st_size == 45_521 and sha(parser) == NOTIFIER_PARSER_SHA
    shutil.copy2(parser, EVIDENCE / 'notifier-external-social-graph.rs')
    write(EVIDENCE / 'notifier-parser.json', dict(source=NOTIFIER_SOURCE, url=parser_url,
        sha256=NOTIFIER_PARSER_SHA, bytes=parser.stat().st_size))
    sys.exit(0)

admission=json.loads((EVIDENCE/'admission.json').read_text()); BASELINE=admission['before']
assert out(['git','rev-parse','HEAD'],cwd=CANDIDATE)==SOURCE
assert not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE)
assert out(['node','--version'])=='v'+NODE and out(['pnpm','--version'])=='10.28.2'
entry=sample();guard(entry);assert entry['available']>=LIMITS['entryAvailableBytes'] and entry['diskFree']>=LIMITS['entryDiskBytes']
if mode == 'transport-replay':
    evidence = dict(fixtureSource=SOURCE, recipe=out(['git','rev-parse','HEAD'],cwd=HERE),
                    passed=False, compilationPerformed=False, releaseQualification=False)
    try:
        run('recover-executables', [sys.executable, HERE / 'stage_transport_replay.py', STAGE, EVIDENCE], 100)
        # The token only authorizes fetching the named retained artifact.
        os.environ.pop('GH_TOKEN')
        retained = json.loads((EVIDENCE / 'replay-inputs.json').read_text())
        binaries = retained['binaries']
        fixture_dir = CANDIDATE / 'rust/crates/nostr-social-graph-server/tests'
        evidence.update(compiledSource=retained['compiledSource'], originalArtifact=retained['artifact'],
                        binaries=binaries, fixtures={n: sha(fixture_dir / n) for n in ['hashtree_publication.mjs', 'snapshot_reader.py']})
        for lock in ['pnpm-lock.yaml', 'package.json', 'ts/package.json']:
            assert sha(CANDIDATE / lock) == retained['dependencyLocks'][lock]
        run('pnpm-frozen', ['pnpm','install','--frozen-lockfile','--ignore-scripts'], 160, cwd=CANDIDATE)
        binary_dir = STAGE / 'replay'
        config = dict(source=str(CANDIDATE),stage=str(STAGE),evidence=str(EVIDENCE),node=shutil.which('node'),
            path=os.environ['PATH'],uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),
            hostPid=os.readlink('/proc/self/ns/pid'),transportReplay=True,
            binary=str(binary_dir / 'nostr-social-graph-server'),reader=str(binary_dir / 'graph-snapshot-reader'),
            readerSha256=binaries['graph-snapshot-reader'],notifier=str(binary_dir / 'notifier-policy-probe'),
            notifierSha256=binaries['notifier-policy-probe'],htree=str(STAGE/'htree'),htreeSha256=HTREE_BINARY_SHA,
            legacyHtree=str(STAGE/'legacy-htree'),legacyHtreeSha256=LEGACY_HTREE_BINARY_SHA)
        write(EVIDENCE/'runtime-input.json',config)
        run('isolated-transport-replay',['sudo','-n','unshare','--mount','--net','--pid','--fork',
            '--kill-child=KILL','--mount-proc',sys.executable,HERE/'namespace.py',EVIDENCE/'runtime-input.json'],450)
        for name, expected in binaries.items():
            assert sha(binary_dir / name) == expected
        assert not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE)
        evidence['passed'] = True
    except BaseException as exc:
        evidence['error'] = repr(exc)
    finally:
        evidence['elapsedSeconds'] = time.monotonic() - START
        write(EVIDENCE/'transport-smoke.json',evidence)
    assert evidence['passed'], evidence.get('error')
    sys.exit(0)

result=dict(source=SOURCE,recipeCommit=out(['git','rev-parse','HEAD'],cwd=HERE),limits=LIMITS,passed=False)
try:
    env=dict(os.environ,CARGO_TARGET_DIR=str(STAGE/'target'),CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',
             CARGO_PROFILE_RELEASE_DEBUG='0',RUSTFLAGS='',CARGO_ENCODED_RUSTFLAGS='',RUSTC_WRAPPER='',RAYON_NUM_THREADS='1')
    run('rust-toolchain',['rustup','toolchain','install','1.99.0','--profile','minimal'],300,env=env)
    cargo=out(['rustup','which','--toolchain','1.99.0','cargo']);rustc=out(['rustup','which','--toolchain','1.99.0','rustc'])
    env['RUSTC']=rustc
    assert 'host: x86_64-unknown-linux-gnu' in out([rustc,'-Vv'])
    assert sha(cargo)=='e951141cc55a6cd7b9876d187bd30a3720e6086b99413af95e3b8de1cdd72f14', 'Unexpected cargo artifact'
    assert sha(rustc)=='f3834d26669b03f6855fa54bd2381443123e860167bc0c7a8dd137e5cf6e5e4f', 'Unexpected rustc artifact'
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
    notifier = EVIDENCE / 'notifier-policy-probe'
    parser = STAGE / 'notifier-probe-source' / 'external_social_graph.rs'
    assert sha(parser) == NOTIFIER_PARSER_SHA
    run('build-notifier-probe', [sys.executable, HERE / 'build_notifier_probe.py',
        rustc, parser, build, notifier], 45, cwd=CANDIDATE / 'rust', env=env)
    notifier_sha = sha(notifier)
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
    run('release-server-build',[cargo,'build','--release','--frozen','-p','nostr-social-graph-server','--bin','nostr-social-graph-server','--bin','graph-snapshot-reader'],1200,cwd=CANDIDATE/'rust',env=env)
    def qualify_executable(name, suffix):
        binary = STAGE / 'target' / 'release' / name
        artifact = EVIDENCE / name
        shutil.copy2(binary, artifact)
        artifact.chmod(0o755)
        assert sha(binary) == sha(artifact)
        libs = out(['ldd', binary])
        (EVIDENCE / ('ldd' + suffix + '.txt')).write_text(libs + '\n')
        paths = set(re.findall(r'(?:=>\s+)?(/[^\s()]+)', libs))
        libraries = {p: dict(sha256=sha(p), bytes=pathlib.Path(p).stat().st_size) for p in sorted(paths)}
        versions = out(['readelf', '--version-info', binary])
        (EVIDENCE / ('elf-versions' + suffix + '.txt')).write_text(versions + '\n')
        glibc = max(tuple(map(int, x.split('.'))) for x in re.findall(r'GLIBC_([0-9.]+)', versions))
        assert glibc <= (2,36), 'Binary exceeds established Debian 12 glibc'
        detail = dict(sha256=sha(artifact), bytes=artifact.stat().st_size,
                      file=out(['file', artifact]), glibcMaximum='.'.join(map(str, glibc)))
        return binary, artifact, detail, libraries
    binary, artifact, writer_info, libraries = qualify_executable('nostr-social-graph-server', '')
    reader, reader_artifact, reader_info, reader_libs = qualify_executable('graph-snapshot-reader', '-reader')
    with open(EVIDENCE/'candidate-source.tar','wb') as f:subprocess.run(['git','archive','--format=tar',SOURCE],cwd=CANDIDATE,stdout=f,check=True,timeout=30)
    locks={p:sha(CANDIDATE/p) for p in ['rust/Cargo.lock','pnpm-lock.yaml','package.json','ts/package.json']}
    result.update(binary=writer_info, readerBinary=reader_info, readerLibraries=reader_libs,
                  toolchain=dict(target='x86_64-unknown-linux-gnu',profile='release',rustc=out([rustc,'-Vv']),rustcSha256=sha(rustc),cargo=out([cargo,'--version']),cargoSha256=sha(cargo),rustup=out(['rustup','--version']),node=out(['node','--version']),nodeSha256=sha(shutil.which('node')),pnpm=out(['pnpm','--version'])),
                  system=dict(runnerImageOS=os.environ.get('ImageOS'),runnerImageVersion=os.environ.get('ImageVersion'),osRelease=pathlib.Path('/etc/os-release').read_text(),kernel=out(['uname','-a']),packages=out(['dpkg-query','-W','libc6','libssl3','libgcc-s1','libstdc++6']),libraries=libraries),
                  locks=locks,sourceArchiveSha256=sha(EVIDENCE/'candidate-source.tar'),harnesses=len(harnesses),
                  oldNegative='Exact old128-author/no-limit wire filter gets100/256+EOSE; frozen legacy prune oracle. No old-binary match claimed.')
    shutil.copytree(HERE,EVIDENCE/'recipe')
    shutil.copy2(HERE.parents[1]/'.github/workflows/graph-server-recovery.yml',EVIDENCE/'recipe-workflow.yml')
    runtime=dict(source=str(CANDIDATE),stage=str(STAGE),evidence=str(EVIDENCE),node=shutil.which('node'),path=env['PATH'],
                 uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),hostPid=os.readlink('/proc/self/ns/pid'),binary=str(binary),harnesses=harnesses,
                 htree=str(STAGE/'htree'),htreeSha256=HTREE_BINARY_SHA,
                 legacyHtree=str(STAGE/'legacy-htree'),legacyHtreeSha256=LEGACY_HTREE_BINARY_SHA,
                 reader=str(reader),readerSha256=reader_info['sha256'],
                 notifier=str(notifier),notifierSha256=notifier_sha)
    write(EVIDENCE/'runtime-input.json',runtime)
    run('isolated-tests',['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',
                          sys.executable,HERE/'namespace.py',EVIDENCE/'runtime-input.json'],900,env=env)
    assert sha(reader_artifact) == sha(reader) == reader_info['sha256']
    assert sha(artifact)==sha(binary),'Candidate binary mutated during tests'
    assert not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE),'Candidate tracked source changed'
    assert all(sha(CANDIDATE/p)==digest for p,digest in locks.items())
    assert sha(notifier) == notifier_sha and sha(parser) == NOTIFIER_PARSER_SHA
    result.update(notifierProbeSha256=notifier_sha,notifierParserSha256=NOTIFIER_PARSER_SHA,passed=True,sourceUnchanged=True,registryPublication=False,deployed=False)
except BaseException as exc:
    result['error']=repr(exc)
finally:
    result.update(phases=[dict(name=p['name'],passed=p['passed'],receipt=p['name']+'.json') for p in PHASES],after=sample(),elapsedSeconds=time.monotonic()-START)
    write(EVIDENCE/'producer-acceptance.json',result)
    print(json.dumps({k:result[k] for k in ['source','passed','elapsedSeconds']}),flush=True)
assert result['passed'],result.get('error')
