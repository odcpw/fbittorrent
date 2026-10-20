#!/usr/bin/env python3
"""Replay real Btrfs shutdown while two bounded private direct-I/O writers run."""
import argparse
import array
import fcntl
import json
import mmap
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from run import REPO, ProcessRun
from daemon import Sandbox, private_environment


def writer(path):
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR | os.O_DIRECT, 0o600)
    buffer = mmap.mmap(-1, 4 * 1024 * 1024)  # page-aligned for O_DIRECT
    try:
        # Btrfs NOCOW on this fresh, empty pressure file prevents a long run
        # accumulating old extent versions. The actual torrent stays COW.
        flags = array.array('L', [0])
        fcntl.ioctl(fd, 0x80086601, flags, True)  # FS_IOC_GETFLAGS
        flags[0] |= 0x00800000  # FS_NOCOW_FL
        fcntl.ioctl(fd, 0x40086602, flags)  # FS_IOC_SETFLAGS
        os.ftruncate(fd, 256 * 1024 * 1024)
        buffer[:] = os.urandom(len(buffer))  # incompressible, bounded storage
        running = True
        def stop(*_):
            nonlocal running
            running = False
        signal.signal(signal.SIGTERM, stop)
        total, began = 0, time.monotonic()
        while running and time.monotonic() - began < 660:
            total += os.pwrite(fd, buffer, total % (256 * 1024 * 1024))
        print(f'private direct writer bytes={total} seconds={time.monotonic()-began:.3f}', flush=True)
    finally:
        buffer.close()
        os.close(fd)


def replay(args):
    kind = subprocess.check_output(['stat', '-f', '-c', '%T', args.disk_root], text=True).strip()
    if kind != 'btrfs':
        raise ValueError('--disk-root must be a Btrfs parent for fresh private directories')
    args.output.mkdir(parents=True, exist_ok=False)
    owners, children, handles, errors = ProcessRun(), [], [], []
    with Sandbox(directory=args.disk_root) as box:
        private_environment(box)  # prove isolation before starting anything
        owners.register(box.root)
        benchmark = None
        try:
            for index in range(2):
                output = (args.output / f'writer-{index}.log').open('w')
                handles.append(output)
                children.append(subprocess.Popen(
                    [sys.executable, __file__, '--worker', str(box.root / f'pressure-{index}.bin')],
                    cwd=box.root, env=box.env, stdout=output, stderr=subprocess.STDOUT,
                    start_new_session=True))
            time.sleep(10)
            if not all(child.poll() is None for child in children):
                raise RuntimeError('private pressure writer failed to start; see writer logs')
            (args.output / 'pressure-before.txt').write_text(Path('/proc/pressure/io').read_text())
            output = (args.output / 'benchmark.log').open('w')
            handles.append(output)
            benchmark = subprocess.Popen([
                sys.executable, str(Path(__file__).with_name('run.py')), '--binary', str(args.binary),
                '--binary-source', args.binary_source, '--seed', str(args.seed),
                '--output', str(args.output / 'runs'), '--disk-root', str(box.root),
                '--runs', '3', '--mib', '1024', '--cases', 'download4'],
                cwd=REPO, env=box.env, stdout=output, stderr=subprocess.STDOUT,
                start_new_session=True)
            if benchmark.wait(timeout=600) != 0:
                raise RuntimeError('Btrfs shutdown replay failed; see benchmark.log')
            (args.output / 'pressure-after.txt').write_text(Path('/proc/pressure/io').read_text())
        except BaseException as error:
            errors.append(error)
        finally:
            # Interrupt the benchmark normally so its own ownership audit runs.
            # The registered outer root also owns every nested daemon sandbox.
            if benchmark is not None and benchmark.poll() is None:
                benchmark.send_signal(signal.SIGINT)
                try:
                    benchmark.wait(timeout=30)
                except subprocess.TimeoutExpired as error:
                    errors.append(error)
            for child in children:
                if child.poll() is None:
                    child.terminate()
            for child in children:
                try:
                    if child.wait(timeout=15) != 0:
                        errors.append(RuntimeError('pressure writer exited unsuccessfully'))
                except subprocess.TimeoutExpired as error:
                    errors.append(error)
            if remaining := owners.survivors():
                errors.append(RuntimeError(f'owned processes required emergency cleanup: {remaining}'))
                try:
                    owners.cleanup(deadline=time.monotonic()+6)
                except BaseException as error:
                    box.retain()  # never dismantle isolation under a live child
                    errors.append(error)
            for child in [*children, *([benchmark] if benchmark is not None else [])]:
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired as error:
                    box.retain()
                    errors.append(error)
            for output in handles:
                output.close()
        if errors:
            raise BaseExceptionGroup('controlled Btrfs replay failed', errors)
    for path in sorted((args.output / 'runs').glob('download4-*/teardown.json')):
        result = json.loads(path.read_text())
        assert result['drained'] and result['survivors'] == 0, result
        print(path.parent.name, f"shutdown_ms={result['shutdown_ms']:.1f}")
    print('PASS: three verified Btrfs payloads, ordered drains, no owned survivors')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', type=Path, help=argparse.SUPPRESS)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--binary-source')
    parser.add_argument('--seed', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--disk-root', type=Path)
    args = parser.parse_args()
    if args.worker:
        return writer(args.worker)
    for field in ('binary', 'binary_source', 'seed', 'output', 'disk_root'):
        if getattr(args, field) is None:
            parser.error('--' + field.replace('_', '-') + ' is required')
    for field in ('binary', 'seed', 'output', 'disk_root'):
        setattr(args, field, getattr(args, field).resolve())
    replay(args)


if __name__ == '__main__':
    main()
