"""Recover three exact release executables for a diagnostic run without compilation."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import urllib.error
import urllib.parse
import urllib.request
import zipfile

ARTIFACT = 14207274214
RUN = 43827651173
RECIPE = 'fa20d5e489a584ed2a0b67ccc4dedb9f52b91aee'
COMPILED_SOURCE = '3dc9ce3d153690cb48d58ce16b1b5279699dedcc'
ARCHIVE_SHA = '4516cddc2efa08bc7c571eb97a1ef31af4baf407a8583d04f988ea2540b93291'
ARCHIVE_BYTES = 43_324_661
SOURCE_ARCHIVE_SHA = '679d132507bf5bd421d12dd81537b150dbbf9f72f676b38f963fbf256980dd4f'
BINARIES = {
    'nostr-social-graph-server': '592cae0bfa1a50a32d320d90aca6a6bfd7e1e384c72573cd726d09c8d974a76d',
    'graph-snapshot-reader': '64bd054963f08437b7d2f91a01c9872702cc6176deed1ac5b930422785fe34d2',
    'notifier-policy-probe': 'a2fffda919afb3fc636b77634f3005cec0d2f66154c255402628424c18d34941',
}


def digest(stream):
    sha = hashlib.sha256()
    while block := stream.read(1024**2):
        sha.update(block)
    return sha.hexdigest()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def main():
    stage, evidence = map(Path, sys.argv[1:3])
    api = f'https://api.github.com/repos/mmalmi/nostr-social-graph/actions/artifacts/{ARTIFACT}'
    headers = {'Authorization': 'Bearer ' + os.environ.pop('GH_TOKEN'), 'Accept': 'application/vnd.github+json'}
    with urllib.request.urlopen(urllib.request.Request(api, headers=headers), timeout=15) as reply:
        descriptor = json.load(reply)
    assert descriptor['id'] == ARTIFACT and descriptor['size_in_bytes'] == ARCHIVE_BYTES
    assert descriptor['digest'] == 'sha256:' + ARCHIVE_SHA and not descriptor['expired']
    assert descriptor['workflow_run']['id'] == RUN and descriptor['workflow_run']['head_sha'] == RECIPE
    request = urllib.request.Request(api + '/zip', headers=headers)
    try:
        urllib.request.build_opener(NoRedirect).open(request, timeout=15)
        raise AssertionError('Expected artifact storage redirect')
    except urllib.error.HTTPError as exc:
        assert exc.code == 302
        url = exc.headers['Location']
    assert urllib.parse.urlparse(url).scheme == 'https'
    archive = stage / 'replay.zip'
    # Only the first-party API receives the token. Do not log signed URLs.
    try:
        with urllib.request.urlopen(url, timeout=30) as response, archive.open('xb') as output:
            count = 0
            sha = hashlib.sha256()
            while block := response.read(1024**2):
                count += len(block)
                assert count <= ARCHIVE_BYTES
                sha.update(block)
                output.write(block)
    except urllib.error.URLError as error:
        raise RuntimeError('Artifact storage read failed') from None
    assert count == ARCHIVE_BYTES and sha.hexdigest() == ARCHIVE_SHA
    retained = stage / 'replay'
    retained.mkdir()
    with zipfile.ZipFile(archive) as packed:
        assert len(packed.namelist()) == len(set(packed.namelist()))
        prior = json.loads(packed.read('producer-acceptance.json'))
        assert prior['source'] == COMPILED_SOURCE and prior['recipeCommit'] == RECIPE
        assert [p['name'] for p in prior['phases'] if not p['passed']] == ['isolated-tests']
        assert prior['binary']['sha256'] == BINARIES['nostr-social-graph-server']
        assert prior['readerBinary']['sha256'] == BINARIES['graph-snapshot-reader']
        assert prior['sourceArchiveSha256'] == SOURCE_ARCHIVE_SHA
        original = json.loads(packed.read('runtime-input.json'))
        assert original['notifierSha256'] == BINARIES['notifier-policy-probe']
        with packed.open('candidate-source.tar') as source:
            assert digest(source) == SOURCE_ARCHIVE_SHA
        for name, expected in BINARIES.items():
            info = packed.getinfo(name)
            assert 0 < info.file_size < 32 * 1024**2 and not info.is_dir()
            target = retained / name
            with packed.open(info) as src, target.open('xb') as dest:
                shutil.copyfileobj(src, dest)
            with target.open('rb') as binary:
                assert digest(binary) == expected
            target.chmod(0o555)
    (evidence / 'replay-inputs.json').write_text(json.dumps(dict(artifact=ARTIFACT, run=RUN,
        archiveSha256=ARCHIVE_SHA, compiledSource=COMPILED_SOURCE, producerRecipe=RECIPE,
        sourceArchiveSha256=SOURCE_ARCHIVE_SHA, binaries=BINARIES, dependencyLocks=prior['locks']), indent=2) + '\n')


if __name__ == '__main__':
    main()
