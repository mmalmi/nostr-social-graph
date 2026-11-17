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
runtime_name=config.get('runtimeName','runtime');assert runtime_name == 'memory'
runtime=evidence/runtime_name;runtime.mkdir()
home=stage/(runtime_name+'-home');home.mkdir();os.chown(home,config['uid'],config['gid'])
os.chown(runtime,config['uid'],config['gid'])
libc=ctypes.CDLL(None,use_errno=True)
libc.prctl.argtypes=[ctypes.c_int,ctypes.c_ulong,ctypes.c_ulong,ctypes.c_ulong,ctypes.c_ulong]
# Bounding-set drops must happen before surrendering root's CAP_SETPCAP.
last_cap=int(pathlib.Path('/proc/sys/kernel/cap_last_cap').read_text())
for capability in range(last_cap+1):
    assert libc.prctl(24,capability,0,0,0)==0,(capability,ctypes.get_errno()) # PR_CAPBSET_DROP
assert libc.prctl(47,4,0,0,0)==0 # PR_CAP_AMBIENT_CLEAR_ALL
os.setgroups([]);os.setgid(config['gid']);os.setuid(config['uid'])
# Explicitly clear inheritable as well as permitted/effective capability words.
class CapHeader(ctypes.Structure):
    _fields_=[('version',ctypes.c_uint32),('pid',ctypes.c_int)]
class CapData(ctypes.Structure):
    _fields_=[('effective',ctypes.c_uint32),('permitted',ctypes.c_uint32),('inheritable',ctypes.c_uint32)]
header=CapHeader(0x20080522,0);data=(CapData*2)()
assert libc.capset(ctypes.byref(header),ctypes.byref(data))==0,ctypes.get_errno()
assert libc.prctl(38,1,0,0,0)==0 # PR_SET_NO_NEW_PRIVS; descendants cannot regain root.
status=dict(line.split(':',1) for line in pathlib.Path('/proc/self/status').read_text().splitlines())
for field in ['CapInh','CapPrm','CapEff','CapBnd','CapAmb']:assert int(status[field],16)==0,(field,status[field])
assert int(status['NoNewPrivs'])==1
capabilities={field:status[field].strip() for field in ['CapInh','CapPrm','CapEff','CapBnd','CapAmb','NoNewPrivs']}
os.environ.clear();os.environ.update(PATH=config['path'],HOME=str(home),TMPDIR=str(home),
    LANG='C.UTF-8',LC_ALL='C.UTF-8',TOKIO_WORKER_THREADS='2',RAYON_NUM_THREADS='1',
    WS_NO_BUFFER_UTIL='1',WS_NO_UTF_8_VALIDATE='1',CI='true')
START=time.monotonic();results=[]
BUDGET=(2 if config['mode']=='paired-capped' else 1)*(1830 if config['tier']=='full' else 930)+20

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
    assert time.monotonic()-START<BUDGET
    log=runtime/(name+'.log');assert not log.exists();start=time.monotonic();child=None;error=None;code=None
    try:
        with log.open('xb') as out:
            child=subprocess.Popen(list(map(str,args)),cwd=cwd,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
            code=child.wait(timeout=min(seconds,BUDGET-(time.monotonic()-START)))
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

result=dict(passed=False,rootDropped=True,noNewPrivileges=True,capabilities=capabilities,netNamespace=os.readlink('/proc/self/ns/net'),pidNamespace=os.readlink('/proc/self/ns/pid'))
try:
    run('network-boundary',[sys.executable,HERE/'network_proof.py',runtime/'network-boundary-proof.json'],15)
    fixture_directory=HERE/('memory-capped' if config['mode']=='paired-capped' else 'memory')
    for label, executable in config['binaries'].items():
        assert sha(executable) == config['hashes'][label]
        command=[config['node'],fixture_directory/'hashtree_publication.mjs',executable,
                 '--htree',stage/'htree','--legacy-htree',stage/'legacy-htree',
                 '--dependencies',source/'ts/package.json','--output',runtime/label,'--memory-seed',config['seed']]
        if config['mode']=='paired-capped':command.extend(['--memory-arenas','2'])
        run('trial-'+label, command, 1800 if config['tier']=='full' else 900, cwd=source)
        trial=json.loads((runtime/label/'receipt.json').read_text())
        assert trial['passed'] and trial['binarySha256']==config['hashes'][label]
    if config['mode']=='paired-capped':
        comparison=runtime/'comparison.json'
        run('compare',[sys.executable,fixture_directory/'compare_memory.py',
                       runtime/'baseline',runtime/'candidate',comparison],30)
        assert json.loads(comparison.read_text())['passed']
    assert not remaining()
    result.update(passed=True,inputs=config['inputs'],tier=config['tier'],mode=config['mode'])
except BaseException as exc:result['error']=repr(exc)
finally:
    result.update(phases=results,remainingProcesses=remaining(),elapsedSeconds=time.monotonic()-START)
    write(runtime/'acceptance.json',result)
    print(json.dumps({k:result[k] for k in ['passed','remainingProcesses','elapsedSeconds']}),flush=True)
assert result['passed'],result.get('error')
