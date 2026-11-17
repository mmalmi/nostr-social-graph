"""Requalify exact prior release binaries against the corrected final JS fixture."""
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

from __main__ import (CANDIDATE, EVIDENCE, HERE, HTREE_BINARY_SHA, LEGACY_HTREE_BINARY_SHA,
                      LIMITS, NOTIFIER_PARSER_SHA, PHASES, SOURCE, STAGE, START, guard,
                      out, run, sample, sha, write)

# Seal these from the finished producer's Actions descriptor and acceptance records.
RUN = 44154914886
RECIPE = 'a9534934bd97eeb1473e0cc334fbd4eef8388080'
COMPILED_SOURCE = '65ddb3605221caff35bcb5e8d579aaca52bc3919'
ALLOWED_CHANGE = 'rust/crates/nostr-social-graph-server/tests/sync_recovery.mjs'


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def restore_archive(pin):
    api = f"https://api.github.com/repos/mmalmi/nostr-social-graph/actions/artifacts/{pin['id']}"
    auth = {'Authorization': 'Bearer ' + os.environ.pop('GH_TOKEN'), 'Accept': 'application/vnd.github+json'}
    with urllib.request.urlopen(urllib.request.Request(api, headers=auth), timeout=15) as reply:
        desc = json.load(reply)
    assert not desc['expired'] and desc['id'] == pin['id'] and desc['size_in_bytes'] == pin['bytes']
    assert desc['workflow_run']['id'] == RUN and desc['workflow_run']['head_sha'] == RECIPE
    assert desc['digest'] == 'sha256:' + pin['sha256']
    try:
        urllib.request.build_opener(NoRedirect).open(urllib.request.Request(api + '/zip', headers=auth), timeout=15)
        raise AssertionError('Expected artifact storage redirect')
    except urllib.error.HTTPError as exc:
        assert exc.code == 302
        url = exc.headers['Location']
    assert urllib.parse.urlparse(url).scheme == 'https'
    dst = STAGE / 'compiled-artifact.zip'
    deadline = time.monotonic() + 90
    try:
        with urllib.request.urlopen(url, timeout=20) as reply, dst.open('xb') as handle:
            digest = hashlib.sha256()
            total = 0
            while block := reply.read(1024**2):
                total += len(block)
                assert total <= pin['bytes'] <= 256 * 1024**2
                assert time.monotonic() <= deadline
                guard(sample())
                digest.update(block)
                handle.write(block)
    except urllib.error.URLError:
        raise RuntimeError('Artifact storage read failed') from None
    assert total == pin['bytes'] and digest.hexdigest() == pin['sha256']
    return dst


def qualify():
    pin = json.loads((HERE / 'recert-input.json').read_text())
    result = dict(source=SOURCE, buildSource=COMPILED_SOURCE, compiledSource=COMPILED_SOURCE, recipeCommit=out(['git','rev-parse','HEAD'], cwd=HERE),
                  compiledRecipe=RECIPE, compiledRun=RUN, compiledArtifact=pin['id'], compiledArtifactSha256=pin['sha256'],
                  compilationPerformed=False, releaseQualification=True, passed=False, limits=LIMITS)
    try:
        diff = out(['git', 'diff', '--name-status', '--no-renames', COMPILED_SOURCE, SOURCE], cwd=CANDIDATE)
        assert diff in ['', 'M\t' + ALLOWED_CHANGE], f'Compiled source changed outside fixture: {diff}'
        assert diff == 'M\t' + ALLOWED_CHANGE
        source_tree = out(['git','rev-parse', SOURCE+'^{tree}'], cwd=CANDIDATE)
        build_tree = out(['git','rev-parse', COMPILED_SOURCE+'^{tree}'], cwd=CANDIDATE)
        tracked = out(['git','ls-tree','-r','--name-only',COMPILED_SOURCE,'rust'], cwd=CANDIDATE).splitlines()
        assert not any(Path(path).name == 'build.rs' for path in tracked)
        metadata = subprocess.run(['git','grep','-n','-I','-E',
            'git (describe|rev-parse)|VERGEN|GIT_(COMMIT|HASH|SHA)|git_version|option_env!|vergen|git-version|shadow-rs|^build[ ]*=',
            COMPILED_SOURCE,'--','rust'], cwd=CANDIDATE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
        assert metadata.returncode == 1, 'Unexpected repository identity or explicit build metadata'
        result.update(sourceTree=source_tree,buildSourceTree=build_tree,buildInputsUnchanged=True,
                      gitIdentityMetadataAbsent=True,fixtureChanges=[ALLOWED_CHANGE], fixtureSha256=sha(CANDIDATE / ALLOWED_CHANGE))
        run('source-history', ['git','fsck','--full','--strict'], 120, cwd=CANDIDATE,
            env={k:v for k,v in os.environ.items() if k!='GH_TOKEN'})
        archive = restore_archive(pin)
        with zipfile.ZipFile(archive) as packed:
            assert len(packed.namelist()) == len(set(packed.namelist()))
            prior = json.loads(packed.read('producer-acceptance.json'))
            old = json.loads(packed.read('runtime-input.json'))
            assert prior['source'] == COMPILED_SOURCE and prior['recipeCommit'] == RECIPE
            assert [p['name'] for p in prior['phases'] if not p['passed']] in ([], ['isolated-tests'])
            assert prior['binary']['sha256'] == pin['server'] and prior['readerBinary']['sha256'] == pin['reader']
            assert old['notifierSha256'] == pin['notifier'] and old['readerSha256'] == pin['reader']
            assert old['htreeSha256'] == HTREE_BINARY_SHA and old['legacyHtreeSha256'] == LEGACY_HTREE_BINARY_SHA
            assert old['source'] == str(CANDIDATE), 'Harness build-time manifest paths differ'
            assert prior['sourceArchiveSha256'] == pin['sourceArchive']
            assert sha(STAGE / 'notifier-probe-source' / 'external_social_graph.rs') == NOTIFIER_PARSER_SHA

            def extract(name, dest, expected=None, executable=False):
                info = packed.getinfo(name)
                assert not info.is_dir() and 0 < info.file_size < 128 * 1024**2
                with packed.open(info) as src, dest.open('xb') as handle:
                    shutil.copyfileobj(src, handle)
                if expected:
                    assert sha(dest) == expected
                if executable:
                    dest.chmod(0o755)

            extract('candidate-source.tar', EVIDENCE / 'compiled-source.tar', pin['sourceArchive'])
            for name, digest in [('nostr-social-graph-server', pin['server']),
                                 ('graph-snapshot-reader', pin['reader']), ('notifier-policy-probe', pin['notifier'])]:
                extract(name, EVIDENCE / name, digest, executable=True)
            write(EVIDENCE / 'compiled-producer.json', prior)
            write(EVIDENCE / 'compiled-runtime.json', json.loads(packed.read('runtime/acceptance.json')))
            extract('runtime/persistence-recovery.log', EVIDENCE / 'compiled-runtime-failure.log')
            for name in ['build-notifier-probe.log', 'cargo-metadata.json', 'ldd.txt', 'elf-versions.txt',
                         'ldd-reader.txt', 'elf-versions-reader.txt', 'release-server-build.json', 'release-test-build.json']:
                extract(name, EVIDENCE / name)
            probe = json.loads((EVIDENCE / 'build-notifier-probe.log').read_text())
            assert probe['binarySha256'] == pin['notifier'] and probe['parserSha256'] == NOTIFIER_PARSER_SHA
            assert probe['driverSha256'] == sha(HERE / 'notifier_probe.rs')
            harnesses = json.loads(packed.read('test-harnesses.json'))
            assert harnesses == old['harnesses'] and len(harnesses) == prior['harnesses'] > 0
            assert len({item['artifact'] for item in harnesses}) == len(harnesses)
            assert any(item['target']['name']=='nostr_social_graph' for item in harnesses)
            assert sum(packed.getinfo(h['artifact']).file_size for h in harnesses) <= 512 * 1024**2
            (EVIDENCE / 'compiled-harnesses').mkdir()
            for item in harnesses:
                member = Path(item['artifact'])
                assert member.parent == Path('compiled-harnesses') and member.name == Path(item['path']).name
                assert Path(item['cwd']).resolve().is_relative_to(CANDIDATE) and Path(item['cwd']).is_dir()
                target = EVIDENCE / member
                extract(str(member), target, item['sha256'], executable=True)
                item['path'] = str(target)
            write(EVIDENCE / 'test-harnesses.json', harnesses)
        # Compare the actual materialized source with the source hash supplied by
        # the producer; hashes in the release receipt distinguish both commits.
        for commit, name in [(COMPILED_SOURCE,'verified-compiled-source.tar'), (SOURCE,'candidate-source.tar')]:
            dest = (STAGE if name == 'verified-compiled-source.tar' else EVIDENCE) / name
            with dest.open('xb') as handle:
                subprocess.run(['git','archive','--format=tar',commit], cwd=CANDIDATE, stdout=handle, check=True, timeout=30)
            if name == 'verified-compiled-source.tar': assert sha(dest) == pin['sourceArchive']
        assert all(sha(CANDIDATE / lock) == digest for lock, digest in prior['locks'].items())
        run('pnpm-frozen',['pnpm','install','--frozen-lockfile','--ignore-scripts'],300,cwd=CANDIDATE)
        binary = EVIDENCE / 'nostr-social-graph-server'
        reader = EVIDENCE / 'graph-snapshot-reader'
        runtime_libraries = {}
        for executable, name in [(binary,'replay-ldd.txt'), (reader,'replay-ldd-reader.txt')]:
            libraries = out(['ldd', executable]); assert 'not found' not in libraries
            (EVIDENCE / name).write_text(libraries + '\n')
            for path in set(re.findall(r'(?:=>\s+)?(/[^\s()]+)',libraries)):
                runtime_libraries[path] = dict(sha256=sha(path),bytes=Path(path).stat().st_size)
        shutil.copytree(HERE,EVIDENCE/'recipe')
        shutil.copy2(HERE.parents[1]/'.github/workflows/graph-server-recovery.yml',EVIDENCE/'recipe-workflow.yml')
        result.update(binary=prior['binary'], readerBinary=prior['readerBinary'], readerLibraries=prior['readerLibraries'],
                      toolchain=prior['toolchain'], compiledSystem=prior['system'], locks=prior['locks'],
                      runtimeSystem=dict(runnerImageOS=os.environ.get('ImageOS'),runnerImageVersion=os.environ.get('ImageVersion'),
                                         osRelease=Path('/etc/os-release').read_text(),kernel=out(['uname','-a']),libraries=runtime_libraries),
                      sourceArchiveSha256=sha(EVIDENCE/'candidate-source.tar'), compiledSourceArchiveSha256=pin['sourceArchive'],
                      notifierProbeSha256=pin['notifier'], notifierParserSha256=NOTIFIER_PARSER_SHA, harnesses=len(harnesses))
        config=dict(source=str(CANDIDATE),stage=str(STAGE),evidence=str(EVIDENCE),node=shutil.which('node'),path=os.environ['PATH'],
                    uid=os.getuid(),gid=os.getgid(),hostNet=os.readlink('/proc/self/ns/net'),hostPid=os.readlink('/proc/self/ns/pid'),
                    binary=str(binary),harnesses=harnesses,htree=str(STAGE/'htree'),htreeSha256=HTREE_BINARY_SHA,
                    legacyHtree=str(STAGE/'legacy-htree'),legacyHtreeSha256=LEGACY_HTREE_BINARY_SHA,
                    reader=str(reader),readerSha256=pin['reader'],notifier=str(EVIDENCE/'notifier-policy-probe'),notifierSha256=pin['notifier'])
        preflight = dict(config, boundaryOnly=True, runtimeName='boundary-preflight', binary='', harnesses=[])
        write(EVIDENCE/'boundary-input.json', preflight)
        run('isolated-preflight',['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',
                                 sys.executable,HERE/'namespace.py',EVIDENCE/'boundary-input.json'],30)
        write(EVIDENCE/'runtime-input.json',config)
        run('isolated-tests',['sudo','-n','unshare','--mount','--net','--pid','--fork','--kill-child=KILL','--mount-proc',
                             sys.executable,HERE/'namespace.py',EVIDENCE/'runtime-input.json'],900)
        assert sha(binary) == pin['server'] and sha(reader) == pin['reader'] and sha(EVIDENCE/'notifier-policy-probe') == pin['notifier']
        assert all(sha(h['path'])==h['sha256'] for h in harnesses)
        assert all(sha(CANDIDATE / lock) == digest for lock,digest in prior['locks'].items())
        assert not out(['git','status','--porcelain','--untracked-files=no'],cwd=CANDIDATE)
        result.update(passed=True,sourceUnchanged=True,registryPublication=False,deployed=False)
    except BaseException as exc:
        result['error'] = repr(exc)
    finally:
        result.update(phases=[dict(name=p['name'],passed=p['passed'],receipt=p['name']+'.json') for p in PHASES],
                      after=sample(),elapsedSeconds=time.monotonic()-START)
        write(EVIDENCE/'producer-acceptance.json',result)
    assert result['passed'],result.get('error')
