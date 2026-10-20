"""Linux x86-64 perf_event_open CPU-clock sampling, without a perf executable.

No signals or injection: each observed task has its own kernel perf ring. Only
user PCs are requested. New threads are discovered every 25 ms; short-lived
threads can escape observation and are explicitly outside this profile.
"""
import ctypes
import fcntl
import json
import mmap
import os
from pathlib import Path
import struct
import threading


class Sampler:
    def __init__(self, pid, output):
        self.pid, self.output = pid, output
        self.events, self.seen, self.pcs, self.errors = {}, set(), [], []
        self.lost = 0
        self.done = threading.Event()
        self.thread = threading.Thread(target=self.poll, name="bench-perf-sampler")
        self.libc = ctypes.CDLL(None, use_errno=True)

    def start(self):
        self.capture_maps()
        self.discover()
        self.thread.start()

    def discover(self):
        for path in Path(f"/proc/{self.pid}/task").iterdir():
            tid = int(path.name)
            if tid in self.seen:
                continue
            self.seen.add(tid)
            attr = bytearray(128)
            # type=SOFTWARE, config=CPU_CLOCK, period=4ms, sample=IP|TID,
            # disabled + exclude_kernel + exclude_hv. No inherited shared ring.
            struct.pack_into("IIQQQQQ", attr, 0, 1, 128, 0, 4_000_000, 3, 0, 1 | 32 | 64)
            cattr = (ctypes.c_char*128).from_buffer(attr)
            fd = self.libc.syscall(298, ctypes.byref(cattr), tid, -1, -1, 0)
            if fd < 0:
                error = ctypes.get_errno()
                if error != 3: self.errors.append(dict(tid=tid, errno=error))
                continue
            ring = mmap.mmap(fd, 4096*9, flags=mmap.MAP_SHARED, prot=mmap.PROT_READ|mmap.PROT_WRITE)
            fcntl.ioctl(fd, 0x2400, 0)  # PERF_EVENT_IOC_ENABLE
            self.events[tid] = (fd, ring)

    def drain(self):
        for fd, ring in self.events.values():
            head, tail, offset, size = struct.unpack_from("QQQQ", ring, 1024)
            while tail < head:
                start = tail % size
                header = bytes(ring[offset+start:offset+min(start+8,size)])
                if len(header)<8: header += ring[offset:offset+8-len(header)]
                kind, _, length = struct.unpack("IHH", header)
                if length < 8 or length > size:
                    raise RuntimeError("invalid perf ring record")
                data = bytes(ring[offset+start:offset+min(start+length,size)])
                if len(data)<length: data += ring[offset:offset+length-len(data)]
                if kind == 9: self.pcs.append(struct.unpack_from("Q",data,8)[0])
                elif kind == 2: self.lost += struct.unpack_from("Q",data,16)[0]
                tail += length
            # x86-64 provides ordered loads and stores for the kernel ring ABI.
            struct.pack_into("Q",ring,1032,tail)

    def capture_maps(self):
        text = Path(f"/proc/{self.pid}/maps").read_text()
        if text:
            self.output.with_suffix(self.output.suffix+".maps").write_text(text)

    def poll(self):
        try:
            while not self.done.wait(.025):
                self.discover()
                self.drain()
                self.capture_maps()
        except (FileNotFoundError, ProcessLookupError):
            pass
        except Exception as error:
            self.errors.append(dict(error=str(error)))

    def stop(self):
        self.done.set()
        self.thread.join(5)
        if self.thread.is_alive(): raise RuntimeError("CPU sampler did not stop")
        for fd, _ in self.events.values(): fcntl.ioctl(fd,0x2401,0)
        self.drain()
        try:
            self.capture_maps()
        except FileNotFoundError:
            pass  # Last live map snapshot is retained, including failed runs.
        for fd, ring in self.events.values():
            ring.close(); os.close(fd)
        self.output.write_text(json.dumps(dict(pcs=self.pcs,lost_samples=self.lost,errors=self.errors,
                                              threads_observed=len(self.events),sample_period_ms=4))+"\n")
