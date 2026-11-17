"""Cold-check the integrated public master from a runner with a separate object store."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

base = "7925d9b54ba7ea306382c0feca7ba3f9c8a58da9"
expected = "83ca2e2a57e145193e0ee7d2273fa5c70457694d"
root = Path(os.environ["RUNNER_TEMP"]) / "graph-source-cold-audit"
root.mkdir()
repo = root / "source.git"
url = "https://github.com/mmalmi/nostr-social-graph.git"
assert shutil.disk_usage(root).free >= 25 * 2**30
baseline_swap = int(next(line.split()[1] for line in Path("/proc/vmstat").read_text().splitlines() if line.startswith("pswpout ")))

def run(*cmd, cwd=None):
    p = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=None)
    while True:
        try:
            result, _ = p.communicate(timeout=1)
            break
        except subprocess.TimeoutExpired:
            if shutil.disk_usage(root).free < 21 * 2**30 or not room():
                p.kill()
                p.wait()
                raise RuntimeError("insufficient host capacity")
    if p.returncode:
        raise RuntimeError(f"{cmd[0]} {cmd[1]} exited {p.returncode}")
    return result

def room():
    available = int(next(line.split()[1] for line in Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemAvailable:")))
    swapout = int(next(line.split()[1] for line in Path("/proc/vmstat").read_text().splitlines() if line.startswith("pswpout ")))
    return available >= 1024**2 and swapout == baseline_swap

assert room()
assert all(key not in os.environ for key in ("GIT_DIR","GIT_ALTERNATE_OBJECT_DIRECTORIES","GIT_OBJECT_DIRECTORY","GIT_SHALLOW_FILE"))
run("git", "clone", "--mirror", "--no-local", "--no-hardlinks", url, str(repo))
def git(*args):
    return run("git", *args, cwd=repo)
assert git("rev-parse", "refs/heads/master").decode().strip() == expected
assert git("rev-parse", "--is-shallow-repository").strip() == b"false"
assert not (repo/"objects/info/alternates").exists() and not (repo/"info/grafts").exists()
assert not git("for-each-ref", "refs/replace")
git("fsck","--full","--strict")
git("merge-base", "--is-ancestor", base, expected)
paths = sorted(git("diff", "--name-only", base, expected).decode().splitlines())
assert paths == sorted(["rust/crates/nostr-social-graph-hashtree/src/lib.rs",
    "rust/crates/nostr-social-graph-hashtree/tests/retention.rs",
    "rust/crates/nostr-social-graph-server/src/lib.rs"])
assert git("rev-parse", "refs/tags/v4.2.1^{}").decode().strip() == base
receipt = {"passed":True,"master":expected,"parent":base,"sdkTag":"v4.2.1",
    "fullHistoryFsck":True,"allRefs":True,"shallow":False,"alternates":False,
    "sha256":{p:hashlib.sha256(git("show",expected+":"+p)).hexdigest() for p in paths},
    "objectCount": len(git("rev-list","--objects","--all").splitlines())}
assert room() and shutil.disk_usage(root).free >= 21 * 2**30
(root.parent/"source-audit.json").write_text(json.dumps(receipt,indent=2)+"\n")
print(json.dumps(receipt))
