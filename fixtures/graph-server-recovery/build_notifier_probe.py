"""Link the unchanged notifier parser to the graph qualification's release libraries."""

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


SOURCE = '32fb561c24bf1ebe078edf4781deabc21e34e1c0'
PARSER_SHA256 = '0ac11030f643a747f8436437984da0cdd5e5606b08d975dda900bde11d740228'
PACKAGES = {
    'registry+https://github.com/rust-lang/crates.io-index#heed@0.20.5': 'heed',
    'registry+https://github.com/rust-lang/crates.io-index#uuid@1.23.4': 'uuid',
}
LMDB_PACKAGE = 'registry+https://github.com/rust-lang/crates.io-index#lmdb-master-sys@0.2.6'


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def release_libraries(log):
    libs = {name: set() for name in PACKAGES.values()}
    lmdb = set()
    with log.open() as stream:
        for line in stream:
            # Cargo's progress lines share the controller's captured output.
            if not line.startswith('{'):
                continue
            event = json.loads(line)
            if event.get('reason') == 'build-script-executed' and event.get('package_id') == LMDB_PACKAGE:
                for linked in event.get('linked_paths', []):
                    if linked.startswith('native='):
                        lmdb.add(Path(linked.removeprefix('native=')) / 'liblmdb.a')
            name = PACKAGES.get(event.get('package_id'))
            if (not name or event.get('reason') != 'compiler-artifact'
                    or event['target']['name'] != name or event['profile']['test']):
                continue
            libs[name].update(Path(p) for p in event['filenames'] if p.endswith('.rlib'))
    result = {}
    for name, paths in libs.items():
        if len(paths) != 1:
            raise ValueError(f'expected one release {name} library, found {len(paths)}')
        path = paths.pop().resolve(strict=True)
        if path.parent.name != 'deps' or path.parent.parent.name != 'release':
            raise ValueError(f'{name} is outside the qualified release profile')
        result[name] = path
    if result['heed'].parent != result['uuid'].parent:
        raise ValueError('release libraries must share the qualified dependency directory')
    if len(lmdb) != 1:
        raise ValueError('expected one native LMDB release library')
    native = lmdb.pop().resolve(strict=True)
    if result['heed'].parent.parent not in native.parents:
        raise ValueError('LMDB is outside the qualified release profile')
    return result, native


def main():
    rustc, parser, log, output = map(Path, sys.argv[1:])
    parser = parser.resolve(strict=True)
    if parser.stat().st_size != 45521 or sha256(parser) != PARSER_SHA256:
        raise ValueError('notifier parser source digest differs')
    if output.exists():
        raise ValueError('choose a fresh probe output path')
    driver = Path(__file__).with_name('notifier_probe.rs')
    libs, native = release_libraries(log)
    command = [rustc, '--edition=2021', '--crate-name', 'graph_notifier_probe',
               '-C', 'opt-level=2', '-L', f'dependency={libs["heed"].parent}',
               '-L', f'native={native.parent}']
    for name, path in libs.items():
        command.extend(['--extern', f'{name}={path}'])
    command.extend([driver, '-o', output])
    subprocess.run(command, env={**os.environ, 'NOTIFIER_PARSER_SOURCE': str(parser)},
                   check=True, timeout=35)
    print(json.dumps({
        'notifierSource': SOURCE, 'parserSha256': PARSER_SHA256,
        'driverSha256': sha256(driver), 'binarySha256': sha256(output),
        'lmdbSha256': sha256(native),
        'libraries': {name: {'file': path.name, 'sha256': sha256(path)}
                      for name, path in libs.items()},
    }))


if __name__ == '__main__':
    main()
