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

SOURCE = '57f6ba33c8c962f3fc19a40eb85d27af8d270b8c'
BASE_SOURCE = '83ca2e2a57e145193e0ee7d2273fa5c70457694d'
BASE_RECIPE = '4e6cd43387f812c461b98c2553d08ea124f34841'
BASE_ARTIFACT = 14302053669
BASE_ARCHIVE_SHA = '1ee73132e5d61fc3435788765fda68a95ed615f26d2c95047930bc581d9e6c43'
BASE_ELF_SHA = '0d6f48e05ca0c88dc530a0f4fbbb49223ed0b17c8eea81046a6a7b8ec3a99040'
DEFAULT_ARTIFACT = 14302419939
DEFAULT_ARCHIVE_SHA = '2bdb7dbc14ebb1c43ff54b248115726baa21340b12bf4606313af9ff3c65ff16'
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


def executable(artifact, source, recipe, digest, elf_sha=None, compiled_only=False):
    assert artifact['name'] == 'graph-server-recovery-' + recipe
    archive = download(artifact, digest)
    path = STAGE / ('server-' + source)
    with zipfile.ZipFile(archive) as files:
        cert = json.loads(files.read('producer-acceptance.json'))
        runtime = json.loads(files.read('runtime/acceptance.json'))
        if compiled_only:
            assert not cert['passed'] and not runtime['passed']
            assert [p['name'] for p in cert['phases'] if not p['passed']] == ['isolated-tests']
            assert [p['name'] for p in runtime['phases'] if not p['passed']] == ['persistence-recovery']
            assert cert['sourceArchiveSha256'] == '3215130171bb56c3ad9cef6645783c140a6653b5834e59edf5f6a0fba18545ed'
        else:
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


mode = sys.argv[1]
assert mode in ('boundary-candidate','memory','paired-daily','full-daily')
recipe = os.environ['GITHUB_SHA']
run_id = os.environ['GITHUB_RUN_ID']
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=SOURCE_DIR, text=True).strip() == SOURCE
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=HERE, text=True).strip() == recipe
assert subprocess.check_output(['node', '--version'], text=True).strip() == 'v22.23.4'
assert subprocess.check_output(['pnpm', '--version'], text=True).strip() == '10.28.2'
subprocess.run(['pnpm', 'install', '--frozen-lockfile', '--ignore-scripts'], cwd=SOURCE_DIR, timeout=350, check=True)
assert api('actions/runs/'+run_id)['head_sha'] == recipe
rows = api('actions/runs/' + run_id + '/artifacts?per_page=100')
assert len(rows['artifacts']) == rows['total_count']
artifact, = [a for a in rows['artifacts'] if a['name'] == 'graph-server-recovery-' + recipe]
assert artifact['workflow_run']['id'] == int(run_id) and artifact['workflow_run']['head_sha'] == recipe
binary, binary_sha = executable(artifact, SOURCE, recipe, artifact['digest'][7:])
binary_source = SOURCE
inputs = dict(artifactId=artifact['id'], archiveSha256=artifact['digest'][7:], source=binary_source, binarySha256=binary_sha)
config = dict(source=str(SOURCE_DIR), stage=str(STAGE), evidence=str(EVIDENCE), path=os.environ['PATH'],
              uid=os.getuid(), gid=os.getgid(), hostNet=os.readlink('/proc/self/ns/net'), hostPid=os.readlink('/proc/self/ns/pid'),
              binary=str(binary), binarySha256=binary_sha, inputs=inputs, node=shutil.which('node'), mode=mode,
              runtimeName='profile-' + mode)
if mode == 'paired-daily':
    prior = api('actions/artifacts/14362023642')
    assert prior['id'] == 14362023642 and prior['workflow_run']['id'] == 44159503711
    old_binary, old_sha = executable(prior, '6e70cd9daf6a9b06d7a8d30ffa32db9d3442d507', '0c1a8a06f40f476f768ae954366f26fa418dd1ec', '4b0d7f8f27f2c60199d9990e523d03eff4cf3659f4b922bdeff86acfb5a29974', '8e58def86557b299075ac070a6a2e56db232eda5baa91ea135529f19d5789466')
    config.update(oldBinary=str(old_binary), oldSha=old_sha, oldArtifact=prior['id'])
if mode == 'memory':
    seed = STAGE / 'full.bin'
    subprocess.run(['node', '--experimental-import-meta-resolve', HERE/'fetch-full.mjs', SOURCE_DIR/'ts/package.json', seed],
                   check=True, timeout=260)
    assert seed.stat().st_size == 55750905 and sha(seed) == 'aeae583ae888f7fec2052f678ff30ae2b031ed4e8d77d0fe0c644e35c2bd6fc0'
    old = api('actions/artifacts/' + str(DEFAULT_ARTIFACT))
    with zipfile.ZipFile(download(old, DEFAULT_ARCHIVE_SHA)) as files:
        baseline = json.loads(files.read('memory/candidate/receipt.json'))
        assert baseline['passed'] and baseline['binarySha256'] == BASE_ELF_SHA and baseline['cycles'] >= 5
        for member in ['receipt.json', 'crawler-memory.json', 'restart-memory.json']:
            (STAGE/('baseline-' + member)).write_bytes(files.read('memory/candidate/' + member))
    config.update(seed=str(seed), baseline=str(STAGE))
before = resources()
assert before['available'] >= (14 if mode in ('memory','full-daily') else 5)*GIB and before['disk'] >= (30 if mode == 'full-daily' else 26)*GIB, before
for name, digest in [('htree', '67306642b1c581d73e89f2d533724f24b877a2b9d5bd59557d522b6f93232850'),
                     ('legacy-htree', 'f3471a3f29d5169cb4e0929cd5554dad9f1b86dabed5b9c3ea3b51b0d071923e')]:
    assert sha(STAGE/name) == digest
save(STAGE/'profile-runtime-input.json', config)
command = ['sudo', '-n', 'unshare', '--mount', '--net', '--pid', '--fork', '--kill-child=KILL', '--mount-proc',
           sys.executable, HERE/'profile_namespace.py', STAGE/'profile-runtime-input.json']
process = subprocess.Popen(command)
error = None
floor = before.copy()
started = time.monotonic()
try:
    while process.poll() is None:
        now = resources()
        for key in ('available', 'disk'):
            floor[key] = min(floor[key], now[key])
        assert now['available'] >= 2*GIB and now['disk'] >= 21*GIB and before['disk']-now['disk'] < (8 if mode == 'full-daily' else 6)*GIB, now
        assert now['swap'] <= before['swap'] and now['swapouts'] == before['swapouts'], now
        assert time.monotonic()-started < (9700 if mode == 'full-daily' else 1900 if mode == 'memory' else 900)
        time.sleep(.5)
    assert process.returncode == 0, process.returncode
except BaseException as exception:
    error = repr(exception)
    if process.poll() is None:
        subprocess.run(['sudo', '-n', 'kill', '-TERM', str(process.pid)], check=False)
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            subprocess.run(['sudo', '-n', 'kill', '-KILL', str(process.pid)], check=False)
            process.wait(timeout=5)
finally:
    save(EVIDENCE/'profile-host.json', dict(source=SOURCE, recipe=recipe, mode=mode, inputs=inputs,
         before=before, floor=floor, after=resources(), passed=error is None, error=error))
if error:
    raise RuntimeError(error)
