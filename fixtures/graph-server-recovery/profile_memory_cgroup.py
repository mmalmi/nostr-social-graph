"""Give the trial an optional cgroup v2 containing only its measured server and children."""
import errno
import os
from pathlib import Path
import re
import sys

ROOT = Path('/sys/fs/cgroup')


def group_path(name):
    assert re.fullmatch(r'graph-profmem-[0-9]+-[0-9]+-(incumbent|reference|candidate)', name)
    return ROOT/name


def prepare(name, uid, gid):
    assert os.getuid()==0 and uid!=0 and gid!=0
    try:
        return _prepare(name,uid,gid)
    except OSError as error:
        # Process-tree RSS/PSS remains independently sampled when this runner lacks a writable memory controller.
        if error.errno in (errno.EROFS, errno.EPERM, errno.EACCES, errno.ENODEV, errno.EOPNOTSUPP):
            return None
        raise


def _prepare(name,uid,gid):
    if not (ROOT/'cgroup.controllers').is_file(): return None
    if 'memory' not in (ROOT/'cgroup.subtree_control').read_text().split(): return None
    root=group_path(name);root.mkdir()
    (root/'cgroup.subtree_control').write_text('+memory')
    fixture=root/'fixture';measured=root/'producer'
    fixture.mkdir();measured.mkdir()
    # The migration permission applies to the common ancestor and the target leaf only.
    for p in [root/'cgroup.procs', measured/'cgroup.procs']:
        os.chown(p,uid,gid)
    (fixture/'cgroup.procs').write_text(str(os.getpid()))
    assert (measured/'memory.current').is_file() and (measured/'memory.stat').is_file()
    return str(measured)


def cleanup(name):
    assert os.getuid()==0
    root=group_path(name)
    if not root.exists():return
    for p in [root/'producer',root/'fixture',root]:
        if p.exists():
            assert not (p/'cgroup.procs').read_text().strip(), 'Task cgroup still populated'
            p.rmdir()


if __name__=='__main__':
    assert len(sys.argv)==3 and sys.argv[1]=='cleanup'
    cleanup(sys.argv[2])
