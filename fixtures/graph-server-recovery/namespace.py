"""PID1 inside a task-only network/mount/PID namespace; drops root before testing."""
import ctypes, hashlib, json, os, pathlib, re, signal, subprocess, sys, time

HERE=pathlib.Path(__file__).resolve().parent
config=json.loads(pathlib.Path(sys.argv[1]).read_text())
assert os.getuid()==0 and os.getpid()==1
assert os.readlink('/proc/self/ns/net')!=config['hostNet']
assert os.readlink('/proc/self/ns/pid')!=config['hostPid']
subprocess.run(['mount','--make-rprivate','/'],check=True,timeout=5)
subprocess.run(['ip','link','set','lo','up'],check=True,timeout=5)
source=pathlib.Path(config['source']);evidence=pathlib.Path(config['evidence']);stage=pathlib.Path(config['stage'])
runtime=evidence/'runtime';runtime.mkdir()
home=stage/'runtime-home';home.mkdir();os.chown(home,config['uid'],config['gid'])
os.chown(runtime,config['uid'],config['gid'])
os.setgroups([]);os.setgid(config['gid']);os.setuid(config['uid'])
assert ctypes.CDLL(None).prctl(38,1,0,0,0)==0 # PR_SET_NO_NEW_PRIVS; descendants cannot regain root.
os.environ.clear();os.environ.update(PATH=config['path'],HOME=str(home),TMPDIR=str(home),
    LANG='C.UTF-8',LC_ALL='C.UTF-8',TOKIO_WORKER_THREADS='2',RAYON_NUM_THREADS='1',
    WS_NO_BUFFER_UTIL='1',WS_NO_UTF_8_VALIDATE='1',CI='true')
START=time.monotonic();results=[]

def sha(path):return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
def write(path,value):path.write_text(json.dumps(value,indent=2)+'\n')
def remaining():
    while True:
        try:
            pid,_=os.waitpid(-1,os.WNOHANG)
            if pid==0:break
        except ChildProcessError:break
    return sorted(int(p.name) for p in pathlib.Path('/proc').iterdir() if p.name.isdigit() and int(p.name)!=1)
def run(name,args,seconds,cwd=None):
    assert time.monotonic()-START<840
    log=runtime/(name+'.log');assert not log.exists();start=time.monotonic();child=None;error=None;code=None
    try:
        with log.open('xb') as out:
            child=subprocess.Popen(list(map(str,args)),cwd=cwd,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
            code=child.wait(timeout=min(seconds,840-(time.monotonic()-START)))
            assert code==0,f'{name} exited{code}'
    except BaseException as exc:error=repr(exc)
    finally:
        if child and child.poll() is None:
            try:os.killpg(child.pid,signal.SIGTERM)
            except ProcessLookupError:pass
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=5)
        extra=remaining()
        if extra:
            error=error or f'Fixture leaked processes:{extra}'
            for pid in extra:
                try:os.kill(pid,signal.SIGKILL)
                except ProcessLookupError:pass
            end=time.monotonic()+5
            while remaining() and time.monotonic()<end:time.sleep(.05)
        record=dict(name=name,args=list(map(str,args)),exitCode=code,passed=error is None,error=error,
                    elapsedSeconds=time.monotonic()-start,remainingProcesses=remaining(),logSha256=sha(log))
        results.append(record);write(runtime/(name+'.json'),record)
        print(json.dumps({k:record[k] for k in ['name','passed','exitCode','elapsedSeconds','error']}),flush=True)
    assert error is None,error
    return log

result=dict(passed=False,rootDropped=True,noNewPrivileges=True,netNamespace=os.readlink('/proc/self/ns/net'),pidNamespace=os.readlink('/proc/self/ns/pid'))
try:
    run('network-boundary',[sys.executable,HERE/'network_proof.py',runtime/'network-boundary-proof.json'],15)
    counts=dict(passed=0,failed=0,ignored=0)
    for i,harness in enumerate(config['harnesses']):
        assert sha(harness['path'])==harness['sha256']
        log=run(f'unit-{i:02d}-{harness["target"]["name"]}',[harness['path'],'--test-threads=1'],180,cwd=harness['cwd'])
        matches=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',log.read_text())
        assert len(matches)==1,'Missing exact harness result'
        for k,n in zip(counts,matches[0]):counts[k]+=int(n)
    core=[h for h in config['harnesses'] if h['target']['name']=='nostr_social_graph' and 'lib' in h['target']['kind']]
    assert len(core)==1
    run('release-prune-benchmark',[core[0]['path'],'--exact','prune_tests::bulk_pruning_sparse_graph_benchmark','--ignored','--nocapture','--test-threads=1'],120,cwd=core[0]['cwd'])
    base=source/'rust/crates/nostr-social-graph-server/tests'
    run('cap-forward-recovery',[config['node'],base/'crawl_cap.mjs',config['binary'],'--dependencies',source/'ts/package.json','--output',runtime/'cap-forward-recovery','--recovery'],165,cwd=source)
    run('persistence-recovery',[config['node'],base/'sync_recovery.mjs',config['binary'],'--dependencies',source/'ts/package.json','--output',runtime/'persistence-recovery'],195,cwd=source)
    cap=json.loads((runtime/'cap-forward-recovery/receipt.json').read_text())
    faults=json.loads((runtime/'persistence-recovery/result.json').read_text())
    assert cap['passed'] and cap['oldFilterCappedHeads']==100 and cap['durableHeads']==256 and cap['recovery']
    assert faults['passed'] and {case['fault'] for case in faults['results']}=={'checkpoint','freshness','allowlist','profile','profile-index'}
    assert not remaining()
    result.update(passed=True,tests=counts,harnesses=len(config['harnesses']),persistenceFaults=5,
                  realCap100=True,forwardOutageRecovery=True,liveAllowlist=True,oldBinaryTested=False,
                  oldNegative='Actual old filter on same capped loopback relay; legacy prune function independently retained as test oracle.')
except BaseException as exc:result['error']=repr(exc)
finally:
    result.update(phases=results,remainingProcesses=remaining(),elapsedSeconds=time.monotonic()-START)
    write(runtime/'acceptance.json',result)
    print(json.dumps({k:result[k] for k in ['passed','remainingProcesses','elapsedSeconds']}),flush=True)
assert result['passed'],result.get('error')
