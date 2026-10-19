"""Linux process cleanup scoped to private fixture directories created by this run.

Do not match executable names or temporary-directory prefixes: other harnesses
and the owner's app may be running concurrently. Never log process environments.
"""
from dataclasses import dataclass
import os
from pathlib import Path
import signal
import stat
import time


ENV_PATHS = {b"HOME", b"TMPDIR", b"XDG_CONFIG_HOME", b"XDG_DATA_HOME",
             b"XDG_RUNTIME_DIR", b"XDG_CACHE_HOME", b"XDG_STATE_HOME"}


@dataclass(frozen=True)
class Process:
    pid: int
    started: str
    name: str
    root: str


def identity(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return fields[19], fields[0]  # starttime, state (a zombie is already dead)


class ProcessRun:
    def __init__(self):
        self.roots = set()
        self.owned = {}

    def register(self, root):
        root = Path(root).resolve(strict=True)
        metadata = root.stat()
        # Callers register freshly created TemporaryDirectory roots, never an
        # inferred HOME/XDG parent. Refuse broad or non-private roots defensively.
        if (root in (Path("/"), Path("/tmp"), Path.home()) or
                metadata.st_uid != os.getuid() or
                stat.S_IMODE(metadata.st_mode) != 0o700 or not root.is_dir()):
            raise ValueError(f"process cleanup requires an owned private fixture directory: {root}")
        self.roots.add(str(root))

    def survivors(self, roots=None):
        roots = self.roots if roots is None else set(map(str, roots))
        if not roots <= self.roots:
            raise ValueError("cannot clean an unregistered fixture directory")
        if not roots:
            return []
        found, processes = {}, {}
        # Keep proven identities through reparenting/environment changes. A
        # child that ignores TERM must still receive KILL after its parent exits.
        for process in self.owned.values():
            if process.root not in roots:
                continue
            try:
                started, state = identity(process.pid)
                if started == process.started and state != "Z":
                    found[process.pid] = process
            except (FileNotFoundError, ProcessLookupError):
                pass
        for path in Path("/proc").iterdir():
            if not path.name.isdecimal() or int(path.name) == os.getpid():
                continue
            try:
                if path.stat().st_uid != os.getuid():
                    continue
                started, state = identity(path.name)
                if state == "Z":
                    continue
                fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
                parent = int(fields[1])
                name = (path / "comm").read_text().strip()
                locations = []
                try:
                    for entry in (path / "environ").read_bytes().split(b"\0"):
                        key, separator, value = entry.partition(b"=")
                        if separator and key in ENV_PATHS:
                            locations.append(os.fsdecode(value))
                except PermissionError:
                    pass
                try:
                    locations.append(os.readlink(path / "cwd").removesuffix(" (deleted)"))
                except PermissionError:
                    pass
                root = next((root for root in sorted(roots) if any(
                    location == root or location.startswith(root + "/") for location in locations)), None)
                current_start, current_state = identity(path.name)
                if current_start == started and current_state != "Z":
                    pid = int(path.name)
                    processes[pid] = (parent, started, name)
                    if root is not None:
                        found[pid] = Process(pid, started, name, root)
            except (FileNotFoundError, ProcessLookupError):
                continue  # exited during the snapshot
        # Children may deliberately replace their environment or working
        # directory. Their ancestry still proves ownership while it is intact.
        while True:
            children = {pid: Process(pid, started, name, found[parent].root)
                        for pid, (parent, started, name) in processes.items()
                        if pid not in found and parent in found}
            if not children:
                self.owned.update({(p.pid, p.started): p for p in found.values()})
                return list(found.values())
            found.update(children)

    @staticmethod
    def send(process, sig):
        # A pidfd plus starttime check prevents signalling a reused PID.
        try:
            fd = os.pidfd_open(process.pid)
        except ProcessLookupError:
            return
        try:
            if identity(process.pid)[0] == process.started:
                signal.pidfd_send_signal(fd, sig)
        except (FileNotFoundError, ProcessLookupError):
            pass
        finally:
            os.close(fd)

    def wait(self, roots=None, seconds=1):
        end = time.monotonic() + seconds
        while True:
            remaining = self.survivors(roots)
            if not remaining or time.monotonic() >= end:
                return remaining
            time.sleep(.05)

    def cleanup(self, roots=None):
        # Rescan after TERM to catch children forked during teardown. A second
        # KILL pass also covers children created just before their parent died.
        for sig, seconds in ((signal.SIGTERM, 3), (signal.SIGKILL, 2), (signal.SIGKILL, 1)):
            for process in self.survivors(roots):
                self.send(process, sig)
            if not self.wait(roots, seconds):
                return
        raise RuntimeError(f"fixture processes survived cleanup: {self.survivors(roots)}")


def finish_cleanup(*actions):
    """Attempt every teardown action even when an earlier action fails."""
    errors = []
    for action in actions:
        try:
            action()
        except BaseException as error:
            errors.append(error)
    if errors:
        raise BaseExceptionGroup("fixture teardown failed", errors)
