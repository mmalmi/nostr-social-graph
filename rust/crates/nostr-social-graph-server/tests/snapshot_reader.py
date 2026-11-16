"""Exercise the release reader with two independently downloaded public generations."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ALT_ROOT = '336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def wait(check, description):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(.1)
    raise AssertionError('Timed out waiting for ' + description)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reader', type=Path)
    parser.add_argument('generations', type=Path)
    parser.add_argument('receipt', type=Path)
    args = parser.parse_args()
    first, second = (args.generations / name for name in ('initial', 'recovered'))
    first_meta, second_meta = (json.loads((p / 'manifest.json').read_bytes()) for p in (first, second))
    root = first_meta['root']
    assert root == second_meta['root']
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        current = work / 'current'
        log = work / 'reader.log'

        def install(generation):
            link = work / 'next'
            link.symlink_to(generation)
            link.replace(current)

        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]

        def request(path, headers=None, method='GET'):
            req = urllib.request.Request(f'http://127.0.0.1:{port}' + path, method=method, headers=headers or {})
            try:
                response = urllib.request.urlopen(req, timeout=2)
            except urllib.error.HTTPError as exc:
                response = exc
            with response:
                return response.status, response.read(), response.headers

        def health(target, status):
            try:
                code, data, headers = request('/healthz')
            except urllib.error.URLError:
                return False
            doc = json.loads(data)
            expected = digest((target / 'manifest.json').read_bytes()) if target else None
            observed = (doc['generation'] or {}).get('id')
            return code == status and doc['healthy'] == (status == 200) and observed == expected and headers['Cache-Control'] == 'no-store'

        def endpoints(generation):
            for route, name in [('/social-graph', 'social-graph.bin'), ('/social-graph/mobile', 'mobile.bin'),
                                ('/profile-index', 'profile-index.json'), ('/allowlist', 'allowlist.txt')]:
                code, data, _ = request(route)
                assert code == 200 and data == (generation / name).read_bytes(), route
            rows = json.loads((generation / 'profile-data.json').read_bytes())
            assert json.loads(request('/profile-data')[1]) == rows
            assert any(len(row) >= 4 for row in rows), 'fixture must include a profile picture'
            no_pictures = [r[:3] if len(r) < 3 or r[2] else r[:2] for r in rows]
            assert json.loads(request('/profile-data?noPictures=true')[1]) == no_pictures
            assert json.loads(request('/profile-data?maxBytes=2')[1]) == []
            assert request('/allowlist?maxDistance=0')[1].decode().splitlines() == [root]
            code, smaller, _ = request('/social-graph?maxNodes=1&maxEdges=1&maxDistance=0&maxEdgesPerNode=1')
            assert code == 200 and smaller[0] == 3 and len(smaller) < (generation / 'social-graph.bin').stat().st_size
            assert request('/social-graph?maxNodes=invalid')[0] == 400
            for mobile_root in [root, ALT_ROOT]:
                path = '/social-graph/mobile?root=' + mobile_root
                code, data, headers = request(path, {'Origin': 'https://example.invalid'})
                assert code == 200 and data[0] == 4 and len(data) <= 4 * 1024 * 1024
                assert headers['x-graph-root'] == mobile_root
                assert headers['ETag'] == f'"{mobile_root}:{digest(data)}"'
                assert headers['Access-Control-Allow-Origin'] == '*'
                assert request(path, {'If-None-Match': headers['ETag']})[:2] == (304, b'')
            assert request('/social-graph/mobile?root=' + 'f' * 64)[0] == 400
            assert request('/social-graph', method='HEAD')[:2] == (200, b'')
            assert request('/social-graph', method='POST')[0] == 405

        env = dict(os.environ, GRAPH_SNAPSHOT_CURRENT=str(current), SOCIAL_GRAPH_ROOT=root,
                   BIND_ADDRESS='127.0.0.1', PORT=str(port), GRAPH_SNAPSHOT_RELOAD_SECS='1',
                   GRAPH_SNAPSHOT_MAX_AGE_SECS='7200', ALLOW_ORIGIN='*', RUST_LOG='info')
        with log.open('wb') as out:
            child = subprocess.Popen([str(args.reader.resolve())], cwd=work, env=env, stdout=out, stderr=out)
        try:
            wait(lambda: health(None, 503), 'empty cold start')
            assert request('/social-graph')[0] == 503
            stale = Path(shutil.copytree(first, work / 'old'))
            old = dict(first_meta, graph_completed_at=1, graph_since=0, generated_at=2)
            (stale / 'manifest.json').write_text(json.dumps(old))
            install(stale)
            wait(lambda: health(stale, 503), 'stale graph despite freshly installed directory')
            assert request('/social-graph')[1] == (first / 'social-graph.bin').read_bytes()
            active = Path(shutil.copytree(first, work / 'first'))
            install(active)
            wait(lambda: health(first, 200), 'healthy first generation')
            endpoints(first)

            partial = Path(shutil.copytree(second, work / 'partial'))
            (partial / 'profile-index.json').unlink()
            position = log.stat().st_size
            install(partial)
            wait(lambda: 'snapshot reload failed' in log.read_text()[position:], 'missing member rejection')
            shutil.rmtree(active)  # Fleet may retire the old directory after switching current.
            endpoints(first)
            assert health(first, 200)
            corrupt = Path(shutil.copytree(second, work / 'corrupt'))
            with (corrupt / 'mobile.bin').open('ab') as handle:
                handle.write(b'truncated-transfer')
            install(corrupt)
            wait(lambda: 'digest mismatch for mobile.bin' in log.read_text(), 'corrupt switch rejection')
            assert health(first, 200)
            endpoints(first)

            install(second.resolve())
            wait(lambda: health(second, 200), 'verified replacement')
            endpoints(second)
            assert (first / 'social-graph.bin').read_bytes() != (second / 'social-graph.bin').read_bytes()
            install(stale)
            wait(lambda: 'moved backwards' in log.read_text(), 'rollback rejection')
            assert health(second, 200)
            endpoints(second)
            assert {p.name for p in work.iterdir()} == {'current', 'old', 'partial', 'corrupt', 'reader.log'}
        except BaseException:
            print(log.read_text()[-2500:])
            raise
        finally:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
    result = dict(passed=True, readerSha256=digest(args.reader.read_bytes()),
                  exactV3=True, mobileRoots=2, retirementPreserved=True, staleHealth=True,
                  corruptRejected=True, incompleteRejected=True, rollbackRejected=True,
                  sourceManifests=[digest((p / 'manifest.json').read_bytes()) for p in (first, second)])
    with args.receipt.open('x') as handle:
        json.dump(result, handle, indent=2)
        handle.write('\n')


if __name__ == '__main__':
    main()
