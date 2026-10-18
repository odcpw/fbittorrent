#!/usr/bin/env python3
"""Run one CI command with retained E.1 logs, including background-worker failures."""
import argparse
import os
import re
import shlex
import subprocess
import sys
import tempfile

from fbtlog import Case, REPO, suite_status

WORKER_FAILURE = re.compile(
    r"thread .*panicked|obligation.*leak|closing native (?:torrent session|runtime):|native torrent session did not drain")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", required=True)
    parser.add_argument("--case", default="command")
    parser.add_argument("--scan-workers", action="store_true")
    parser.add_argument("--isolate-state", action="store_true", help="isolate XDG state for older namespace fixtures")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command after -- is required")
    with tempfile.TemporaryDirectory(prefix="fbt-ci-state-") as state, Case(args.suite, args.case) as case:
        stdout_path = case.path.with_suffix(".stdout.log")
        step = case.step("run " + shlex.join(command), "command exits 0" +
                         (" without background worker failures" if args.scan_workers else ""))
        child = case.popen(command, cwd=REPO, stdout=subprocess.PIPE, text=True,
                           errors="replace", env=dict(os.environ, XDG_STATE_HOME=state, TMPDIR=state)
                           if args.isolate_state else None)
        with stdout_path.open("w") as output:
            for line in child.stdout:
                output.write(line)
                print(line, end="", flush=True)
        code = child.wait()
        stderr = case.suite.stderr_path.read_bytes()[case.stderr_offset:].decode(errors="replace")
        failures = WORKER_FAILURE.findall(stdout_path.read_text() + "\n" + stderr) if args.scan_workers else []
        step.close(code == 0 and not failures,
                   f"Exit {code}; background worker failures: {failures}.",
                   artifacts=[stdout_path, case.daemon_path])
    return suite_status(args.suite)


if __name__ == "__main__":
    sys.exit(main())
