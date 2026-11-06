"""Qualify the built SDK in an immutable browser image, without network access."""
import json
import os
from pathlib import Path
import subprocess
import signal
import uuid

IMAGE = 'mcr.microsoft.com/playwright@sha256:cf0daee9b994042e011bc29f20cdff1a9f682a039b43fcd738f7d8a9d3bcd9d6'
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
EVIDENCE = Path(os.environ['GRAPH_GATE_EVIDENCE']).resolve()
NAME = 'graph-sdk-' + uuid.uuid4().hex[:16]
record = {'name': NAME, 'image': IMAGE, 'source': os.environ['GITHUB_SHA']}


def docker(*args, timeout=30):
    return subprocess.check_output(['docker', *args], text=True, timeout=timeout)


def inspect():
    return json.loads(docker('inspect', NAME))[0]


assert os.getuid() != 0
def interrupted(number, _frame):
    raise SystemExit(128 + number)
signal.signal(signal.SIGTERM, interrupted)
signal.signal(signal.SIGINT, interrupted)
try:
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    record['imageIdentity'] = json.loads(docker('image', 'inspect', IMAGE))[0]['Id']
    assert (EVIDENCE / 'build-runtime.json').is_file(), 'Missing Node22 production build receipt'
    user = f'{os.getuid()}:{os.getgid()}'
    # NAME is recorded before create: cleanup covers an interrupted create CLI.
    docker('create', '--name', NAME, '--pull=never', '--network=none', '--ipc=private',
           '--user', user, '--cap-drop=ALL', '--security-opt=no-new-privileges',
           '--read-only', '--cpus=2', '--memory=2g', '--memory-swap=2g',
           '--pids-limit=256', '--shm-size=256m',
           '--tmpfs', '/tmp:rw,nosuid,nodev,size=536870912,mode=1777',
           '--mount', f'type=bind,source={ROOT},target=/work',
           '--mount', f'type=bind,source={EVIDENCE},target=/evidence',
           '--workdir', '/work', '--env', 'CI=true', '--env', 'HOME=/tmp/graph-home',
           '--env', 'GRAPH_GATE_EVIDENCE=/evidence',
           '--env', 'GRAPH_HOST_NETNS=' + os.readlink('/proc/self/ns/net'),
           '--env', 'GRAPH_HOST_PIDNS=' + os.readlink('/proc/self/ns/pid'),
           '--env', 'PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1',
           '--env', 'PLAYWRIGHT_BROWSERS_PATH=/ms-playwright',
           IMAGE, '/usr/bin/timeout', '--signal=TERM', '--kill-after=10s', '420s',
           'node', '/work/.github/scripts/graph-gates/browser.cjs')
    info = inspect()
    host = info['HostConfig']
    assert info['Image'] == record['imageIdentity']
    assert info['Config']['User'] == user
    assert host['NetworkMode'] == 'none' and host['PidMode'] == ''
    assert host['IpcMode'] == 'private' and not host['Privileged']
    assert host['CapDrop'] == ['ALL'] and 'no-new-privileges' in host['SecurityOpt']
    assert host['ReadonlyRootfs'] and host['Memory'] == 2 * 1024**3
    assert host['MemorySwap'] == host['Memory'] and host['PidsLimit'] == 256
    assert host['NanoCpus'] == 2_000_000_000
    record['policy'] = {k: host[k] for k in ['NetworkMode', 'PidMode', 'IpcMode',
        'Privileged', 'CapDrop', 'SecurityOpt', 'ReadonlyRootfs', 'Memory', 'MemorySwap', 'PidsLimit']}
    docker('start', NAME)
    # An independent in-container timeout survives controller cancellation.
    result = int(docker('wait', NAME, timeout=450).strip())
    record['exitCode'] = result
    terminal = inspect()['State']
    assert not terminal['Running'] and terminal['Pid'] == 0
    record['terminal'] = terminal
    assert result == 0, f'Isolated browser gate exited {result}'
    runtime = json.loads((EVIDENCE / 'browser-runtime.json').read_text())
    assert runtime['passed'] and runtime['unsignedProofPassed']
    record['passed'] = True
except BaseException as error:
    record['error'] = repr(error)
    raise
finally:
    try:
        logs = subprocess.run(['docker', 'logs', NAME], capture_output=True, timeout=15)
        (EVIDENCE / 'browser.log').write_bytes(logs.stdout + logs.stderr)
        print((logs.stdout + logs.stderr).decode(errors='replace')[-4000:])
    finally:
        removed = subprocess.run(['docker', 'rm', '--force', NAME], capture_output=True, text=True, timeout=30)
        record['cleanup'] = {'exitCode': removed.returncode, 'stdout': removed.stdout, 'stderr': removed.stderr}
        # Success requires removal of the exact owned container; failures retain
        # diagnostics and cannot become an accepted gate.
        cleanup_failed = record.get('passed') and removed.returncode != 0
        if cleanup_failed:
            record.update(passed=False, error='Container cleanup failed')
        (EVIDENCE / 'container.json').write_text(json.dumps(record, indent=2) + '\n')
        assert not cleanup_failed, 'Container cleanup failed'
