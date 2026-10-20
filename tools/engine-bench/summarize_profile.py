#!/usr/bin/env python3
"""Symbolize CPU-time PC samples from profile.c; no suspended async stacks.

The denominator includes unresolved and library PCs. This is exclusive CPU
attribution, not wall time or an inclusive call graph. Count runs are diagnostic.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import subprocess
import struct

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("counters", type=Path)
p.add_argument("--binary", type=Path, required=True)
p.add_argument("--bytes", type=int, required=True)
args = p.parse_args()
data = json.loads(args.counters.read_text())
assert not data["errors"] and data["lost_samples"] == 0, {k:v for k,v in data.items() if k!="pcs"}
maps = []
for line in Path(str(args.counters)+".maps").read_text().splitlines():
    fields = line.split(maxsplit=5)
    if len(fields) == 6 and "x" in fields[1]:
        start, end = (int(x,16) for x in fields[0].split("-"))
        maps.append((start,end,int(fields[2],16),fields[5]))
def virtual_offset(path, file_offset):
    # PT_LOAD virtual addresses need not equal file offsets (Rust/lld inserts
    # segment alignment gaps). Resolve the actual executable segment first.
    if not Path(path).is_file(): return file_offset
    with Path(path).open("rb") as binary:
        header = binary.read(64)
        if header[:6] != b"\x7fELF\x02\x01": raise ValueError("expected ELF64 little endian")
        phoff = struct.unpack_from("Q",header,32)[0]
        entry_size, count = struct.unpack_from("HH",header,54)
        binary.seek(phoff)
        for index in range(count):
            entry = binary.read(entry_size)
            kind, flags, offset, address, _, size, _, _ = struct.unpack_from("IIQQQQQQ",entry)
            if kind == 1 and flags & 1 and offset//4096 == file_offset//4096:
                return address + file_offset - offset
    raise ValueError(f"no executable segment for {path} offset {file_offset:x}")

counts = Counter(data.pop("pcs"))
by_file = defaultdict(dict)
for pc, count in counts.items():
    mapping = next((m for m in maps if m[0]<=pc<m[1]), None)
    if mapping:
        start, _, offset, path = mapping
        # Remote-built DWARF paths do not affect ELF addresses. Use the exact
        # measured binary even if its process path was temporary.
        if Path(path).name == args.binary.name:
            path = str(args.binary.resolve())
        by_file[path][pc-start+virtual_offset(path, offset)] = count
    else:
        by_file["unresolved"][pc] = count
symbols = Counter()
for path, addresses in by_file.items():
    if not Path(path).is_file():
        symbols[path] += sum(addresses.values())
        continue
    items = list(addresses)
    for begin in range(0,len(items),1000):
        batch = items[begin:begin+1000]
        result = subprocess.check_output(["addr2line","-f","-C","-e",path,*map(hex,batch)],text=True).splitlines()
        for index, address in enumerate(batch):
            symbol = result[index*2]
            if symbol == "??": symbol = Path(path).name + ":unresolved"
            symbols[symbol] += addresses[address]
total = sum(counts.values())
allocation_file = args.counters.parent / "allocations.json.live"
allocations, requested_bytes = struct.unpack("QQ", allocation_file.read_bytes()) if allocation_file.exists() else (None,None)
result = dict(**data, samples=total, allocations=allocations, requested_bytes=requested_bytes,
              allocations_mb=allocations/(args.bytes/1e6) if allocations is not None else None,
              attribution="exclusive user PC, per-thread kernel CPU clock; separate instrumented run",
              hotspots=[dict(symbol=symbol,samples=count,percent=round(100*count/max(1,total),2)) for symbol,count in symbols.most_common(25)])
print(json.dumps(result,indent=2))
