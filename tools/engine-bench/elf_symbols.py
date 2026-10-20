"""Bounded ELF function ranges; a nearby exported name is not attribution."""
from bisect import bisect_right
import mmap
import struct


class FunctionRanges:
    def __init__(self, path):
        by_start = {}
        with open(path, 'rb') as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as elf:
            if elf[:6] != b'\x7fELF\x02\x01':
                raise ValueError('expected ELF64 little endian')
            offset = struct.unpack_from('Q', elf, 40)[0]
            stride, count = struct.unpack_from('HH', elf, 58)
            sections = [struct.unpack_from('IIQQQQIIQQ', elf, offset+i*stride) for i in range(count)]
            for section in sections:
                if section[1] not in (2, 11):  # SHT_SYMTAB, SHT_DYNSYM
                    continue
                strings = sections[section[6]]
                for i in range(section[5]//section[9]):
                    name, info, _, index, address, size = struct.unpack_from(
                        'IBBHQQ', elf, section[4]+i*section[9])
                    if info & 15 not in (2, 10) or not index or not size:
                        continue  # STT_FUNC/GNU_IFUNC with a real extent
                    end = address+size
                    if address in by_start and by_start[address][0] >= end:
                        continue
                    start = strings[4]+name
                    symbol = elf[start:elf.find(b'\0', start)].decode(errors='replace')
                    by_start[address] = end, symbol
        self.starts = sorted(by_start)
        self.ranges = [by_start[start] for start in self.starts]

    def containing(self, address):
        index = bisect_right(self.starts, address)-1
        if index >= 0 and address < self.ranges[index][0]:
            return self.ranges[index][1]
        return None
