"""Run graph fixtures with inherited, non-root network/PID isolation on Linux."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inner(config):
    env = config['env']
    evidence = Path(env['GRAPH_GATE_EVIDENCE'])
    assert os.getuid() != 0
    subprocess.run([sys.executable, str(HERE / 'network-proof.py'),
                    str(evidence / (config['mode'] + '-network.json'))], check=True, timeout=15)
    record = {'mode': config['mode'], 'source': env['GITHUB_SHA'], 'commands': [],
              'node': subprocess.check_output(['node', '--version'], text=True).strip(),
              'pnpm': subprocess.check_output(['pnpm', '--version'], text=True).strip(),
              'lockSha256': digest(ROOT / 'pnpm-lock.yaml')}
    try:
        assert record['node'] == 'v22.23.4'
        assert record['pnpm'] == '10.28.2'
        if config['mode'] == 'tests':
            record['rustc'] = subprocess.check_output(['rustc', '--version'], text=True).strip()
            record['cargo'] = subprocess.check_output(['cargo', '--version'], text=True).strip()
            assert record['rustc'].startswith('rustc 1.99.0 ')
            assert record['cargo'].startswith('cargo 1.99.0 ')
            record['cargoLockSha256'] = digest(ROOT / 'rust/Cargo.lock')
            commands = [
                ['pnpm', '--filter', 'nostr-social-graph', 'exec', 'vitest', 'run'],
                ['cargo', 'test', '--manifest-path', 'rust/Cargo.toml', '--locked', '--offline',
                 '-j1', '-p', 'nostr-social-graph-hashtree', '-p', 'nostr-social-graph-server'],
            ]
        else:
            commands = [['pnpm', '--filter', 'nostr-social-graph', 'build']]
        for index, command in enumerate(commands):
            log = evidence / f"{config['mode']}-{index}.log"
            with log.open('xb') as output:
                result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
            record['commands'].append({'argv': command, 'exitCode': result.returncode, 'log': log.name})
            print(log.read_text(errors='replace')[-4000:], flush=True)
            assert result.returncode == 0, f'{command[0]} failed: {result.returncode}'
        if config['mode'] == 'build':
            record['productionDist'] = {p.relative_to(ROOT / 'ts/dist').as_posix(): digest(p)
                                        for p in sorted((ROOT / 'ts/dist').rglob('*')) if p.is_file()}
            assert record['productionDist'], 'Production build emitted no files'
        record['passed'] = True
    except BaseException as error:
        record['error'] = repr(error)
        raise
    finally:
        save(evidence / (config['mode'] + '-runtime.json'), record)


if __name__ == '__main__':
    mode = sys.argv[1]
    if mode in ('tests', 'build'):
        assert os.getuid() != 0, 'Invoke from the unprivileged runner'
        evidence = Path(os.environ['GRAPH_GATE_EVIDENCE']).resolve()
        evidence.mkdir(parents=True, exist_ok=True)
        home = evidence / 'home'
        home.mkdir(exist_ok=True)
        env = dict(PATH=os.environ['PATH'], HOME=str(home), TMPDIR='/tmp', CI='true',
                   LANG='C.UTF-8', GRAPH_GATE_EVIDENCE=str(evidence),
                   GRAPH_HOST_NETNS=os.readlink('/proc/self/ns/net'),
                   GRAPH_HOST_PIDNS=os.readlink('/proc/self/ns/pid'),
                   GRAPH_HOST_MNTNS=os.readlink('/proc/self/ns/mnt'),
                   GITHUB_SHA=os.environ['GITHUB_SHA'],
                   CARGO_HOME=os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')),
                   RUSTUP_HOME=os.environ.get('RUSTUP_HOME', str(Path.home() / '.rustup')),
                   RUSTUP_TOOLCHAIN='1.99.0', CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='1',
                   PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD='1', COREPACK_ENABLE_NETWORK='0')
        config = dict(mode=mode, uid=os.getuid(), gid=os.getgid(), env=env)
        config_path = evidence / (mode + '-config.json')
        save(config_path, config)
        # The PID namespace is killed when its init exits, including timeout or
        # cancellation; unshare also kills its child if the outer owner exits.
        command = ['sudo', '/usr/bin/timeout', '--signal=TERM', '--kill-after=10s',
                   '1200s' if mode == 'tests' else '180s', '/usr/bin/unshare',
                   '--mount', '--net', '--pid', '--fork', '--mount-proc',
                   '--propagation', 'private', '--kill-child=KILL',
                   '/usr/bin/python3', str(HERE / 'host.py'), 'root', str(config_path)]
        result = subprocess.run(command, cwd=ROOT)
        save(evidence / (mode + '-exit.json'), {'exitCode': result.returncode})
        sys.exit(result.returncode)
    elif mode == 'root':
        config = json.loads(Path(sys.argv[2]).read_text())
        assert os.getuid() == 0
        assert os.readlink('/proc/self/ns/net') != config['env']['GRAPH_HOST_NETNS']
        assert os.readlink('/proc/self/ns/pid') != config['env']['GRAPH_HOST_PIDNS']
        assert os.readlink('/proc/self/ns/mnt') != config['env']['GRAPH_HOST_MNTNS']
        links = json.loads(subprocess.check_output(['/usr/sbin/ip', '-j', 'link', 'show'], text=True))
        assert [item['ifname'] for item in links] == ['lo']
        # Mount a read-only sysfs view for this network namespace only.
        subprocess.run(['/usr/bin/mount', '-t', 'sysfs', '-o', 'ro,nosuid,nodev,noexec', 'sysfs', '/sys'], check=True)
        subprocess.run(['/usr/sbin/ip', 'link', 'set', 'lo', 'up'], check=True)
        os.execve('/usr/bin/setpriv', ['setpriv', '--reuid=' + str(config['uid']),
                  '--regid=' + str(config['gid']), '--clear-groups', '--bounding-set=-all',
                  '--inh-caps=-all', '--ambient-caps=-all', '--no-new-privs',
                  '/usr/bin/python3', str(HERE / 'host.py'), 'inner', sys.argv[2]], config['env'])
    elif mode == 'inner':
        inner(json.loads(Path(sys.argv[2]).read_text()))
    else:
        raise ValueError('Unsupported gate mode')
