"""Stage pinned executables online, then qualify them inside an isolated Linux namespace."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import zipfile

HERE = Path(__file__).resolve().parent
STAGE = Path(os.environ['RUNNER_TEMP']) / 'graph-server-alias-repair'
SOURCE_DIR = Path(os.environ['GITHUB_WORKSPACE']) / 'candidate'
EVIDENCE = STAGE / 'evidence'
GIB = 2**30


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(2**20), b''):
            digest.update(block)
    return digest.hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def api(path):
    return json.loads(subprocess.check_output(['gh', 'api', 'repos/mmalmi/nostr-social-graph/' + path], timeout=60))


def download(artifact, expected_sha):
    assert not artifact['expired'] and artifact['digest'] == 'sha256:' + expected_sha
    assert artifact['size_in_bytes'] < 150*2**20
    path = STAGE / ('artifact-' + str(artifact['id']) + '.zip')
    with path.open('xb') as output:
        subprocess.run(['gh', 'api', f"repos/mmalmi/nostr-social-graph/actions/artifacts/{artifact['id']}/zip"],
                       stdout=output, timeout=180, check=True)
    assert path.stat().st_size == artifact['size_in_bytes'] and sha(path) == expected_sha
    return path


def executable(artifact, source, recipe, digest, elf_sha=None):
    assert artifact['name'] == 'graph-server-recovery-' + recipe
    archive = download(artifact, digest)
    path = STAGE / ('server-' + source)
    with zipfile.ZipFile(archive) as files:
        cert = json.loads(files.read('producer-acceptance.json'))
        runtime = json.loads(files.read('runtime/acceptance.json'))
        assert cert['passed'] and cert['sourceUnchanged'] and runtime['passed']
        assert cert['source'] == source and cert['recipeCommit'] == recipe
        assert elf_sha is None or cert['binary']['sha256'] == elf_sha
        member = files.getinfo('nostr-social-graph-server')
        assert member.file_size == cert['binary']['bytes'] and 1_000_000 < member.file_size < 40_000_000
        with files.open(member) as incoming, path.open('xb') as output:
            shutil.copyfileobj(incoming, output)
    assert sha(path) == cert['binary']['sha256']
    path.chmod(0o555)
    return path, cert['binary']['sha256']


def resources():
    memory = {key: int(value.split()[0])*1024 for key, value in
              (line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())}
    vm = dict(line.split() for line in Path('/proc/vmstat').read_text().splitlines())
    return dict(available=memory['MemAvailable'], swap=memory['SwapTotal']-memory['SwapFree'],
                swapouts=int(vm['pswpout']), disk=shutil.disk_usage(STAGE).free)


def guard(before, floor):
    now = resources()
    for key in ('available', 'disk'):
        floor[key] = min(floor[key], now[key])
    assert now['available'] >= 2*GIB and now['disk'] >= 21*GIB and before['disk'] - now['disk'] < 6*GIB, 'Runner reserve'
    assert now['swap'] <= before['swap'] and now['swapouts'] == before['swapouts'], 'Runner swap'
    return now


attested = json.loads((HERE/'profile-memory-pins.json').read_text())
recipe = os.environ['GITHUB_SHA']
run_id = os.environ['GITHUB_RUN_ID']; attempt = os.environ['GITHUB_RUN_ATTEMPT']
assert run_id.isdecimal() and attempt.isdecimal()
assert api('actions/runs/'+run_id)['head_sha'] == recipe
for path, expected in [(SOURCE_DIR, attested['source']), (HERE, recipe)]:
    assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=path,text=True).strip() == expected
public = EVIDENCE/'profile-memory-aggregate'
public.mkdir()
before = resources(); floor = before.copy()
assert before['available'] >= 14*GIB and before['disk'] >= 28*GIB, before
started = time.monotonic(); stages = []; active = 'dependencies'
outcome = dict(passed=False, inputs=attested, recipe=recipe, run=int(run_id), before=before)
try:
    assert subprocess.check_output(['node','--version'],text=True).strip() == 'v22.23.4'
    assert subprocess.check_output(['pnpm','--version'],text=True).strip() == '10.28.2'
    subprocess.run(['pnpm','install','--frozen-lockfile','--ignore-scripts'],cwd=SOURCE_DIR,check=True,timeout=350)
    for name, digest in [('htree','67306642b1c581d73e89f2d533724f24b877a2b9d5bd59557d522b6f93232850'),
                         ('legacy-htree','f3471a3f29d5169cb4e0929cd5554dad9f1b86dabed5b9c3ea3b51b0d071923e')]:
        assert sha(STAGE/name) == digest
    stages.append(dict(name=active,elapsedSeconds=time.monotonic()-started))
    active = 'input-verification'; stage_start=time.monotonic()
    frozen = STAGE/'profile-input'
    frozen.mkdir(mode=0o700)
    subprocess.run(['node','--experimental-import-meta-resolve',HERE/'fetch-full-profile.mjs',SOURCE_DIR/'ts/package.json',frozen],
                   check=True, timeout=510)
    assert sorted(m.name for m in frozen.iterdir()) == sorted(attested['members'])
    for name, pin in attested['members'].items():
        assert (frozen/name).stat().st_size == pin['size'] and sha(frozen/name) == pin['sha256']
    guard(before,floor)
    stages.append(dict(name=active,elapsedSeconds=time.monotonic()-stage_start))
    trials = {}
    for label, pin in attested['binaries'].items():
        assert label in ('incumbent','reference','candidate')
        active = label+'-executable';stage_start=time.monotonic()
        artifact=api('actions/artifacts/'+str(pin['archiveId']))
        assert artifact['id'] == pin['archiveId'] and artifact['size_in_bytes'] == pin['archiveBytes']
        assert artifact['workflow_run']['id'] == pin['run'] and artifact['workflow_run']['head_sha'] == pin['recipe']
        binary, elf = executable(artifact,pin['source'],pin['recipe'],pin['archiveSha256'],pin['binarySha256'])
        stages.append(dict(name=active,elapsedSeconds=time.monotonic()-stage_start))
        active = label+'-trial';stage_start=time.monotonic()
        # Re-read each file before each process starts, warming both inputs equivalently.
        for name,p in attested['members'].items(): assert sha(frozen/name) == p['sha256']
        name = 'profile-'+label; group = 'graph-profmem-'+run_id+'-'+attempt+'-'+label
        config = dict(source=str(SOURCE_DIR),stage=str(STAGE),evidence=str(EVIDENCE),path=os.environ['PATH'],
            uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),hostPid=os.readlink('/proc/self/ns/pid'),
            binary=str(binary),binarySha256=elf,inputs=pin,mode=label,runtimeName=name,node=shutil.which('node'),
            fixtureInput=str(frozen),cgroupName=group,members=attested['members'])
        conf = STAGE/(name+'-config.json'); save(conf,config)
        command=['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',
                 sys.executable,HERE/'profile_memory_namespace.py',conf]
        process = subprocess.Popen(command)
        try:
            while process.poll() is None:
                guard(before,floor)
                assert time.monotonic()-stage_start < 1900, 'Trial deadline'
                time.sleep(.35)
            assert process.returncode == 0,'Isolated memory trial failed'
        finally:
            if process.poll() is None:
                subprocess.run(['sudo','-n','kill','-TERM',str(process.pid)],check=False)
                try: process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    subprocess.run(['sudo','-n','kill','-KILL',str(process.pid)],check=False)
                    process.wait(timeout=5)
            subprocess.run(['sudo','-n',sys.executable,HERE/'profile_memory_cgroup.py','cleanup',group],check=True,timeout=15)
            diagnostics=EVIDENCE/name/'safe-failure.json'
            if diagnostics.exists():shutil.copy2(diagnostics,public/(label+'-failure.json'))
        trial=json.loads((EVIDENCE/name/'candidate'/'receipt.json').read_bytes())
        assert trial['passed'] and trial['binarySha256'] == elf and trial['cycles'] >= 5
        ns=json.loads((EVIDENCE/name/'acceptance.json').read_bytes())
        assert ns['passed'] and ns['rootDropped'] and ns['noNewPrivileges'] and not ns['remainingProcesses']
        assert all(p['passed'] and p['exitCode'] == 0 and not p['remainingProcesses'] for p in ns['phases'])
        assert not any(int(v,16) for k,v in ns['capabilities'].items() if k.startswith('Cap'))
        save(public/(label+'-isolation.json'),dict(passed=True,isolated=True,clean=True,
            stages=[dict(name=p['name'],elapsedSeconds=p['elapsedSeconds']) for p in ns['phases']]))
        save(public/(label+'-trial.json'),trial)
        trials[label]=trial
        stages.append(dict(name=active,elapsedSeconds=time.monotonic()-stage_start))
    if 'candidate' in trials:
        active='paired-equality'
        original=trials['reference']; updated=trials['candidate']
        for field in ['sourceRows','inputDigests','fileHashes','checkpoint']:
            assert updated[field] == original[field],field
        assert len(updated['fileHashes']) == 7
    outcome['passed'] = True
finally:
    outcome.update(floor=floor,after=resources(),stages=stages,elapsedSeconds=time.monotonic()-started)
    if not outcome['passed']: outcome['failedStage']=active
    save(public/'host-acceptance.json',outcome)
