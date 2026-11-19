"""Pack PNG files into a .ico (PNG-compressed entries, Vista+)."""
import struct
import sys
from pathlib import Path

out, *pngs = sys.argv[1:]
blobs = [Path(p).read_bytes() for p in pngs]
sizes = [struct.unpack(">II", b[16:24]) for b in blobs]
head = struct.pack("<HHH", 0, 1, len(blobs))
off = 6 + 16 * len(blobs)
dirs = b""
for (w, h), b in zip(sizes, blobs):
    dirs += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(b), off)
    off += len(b)
Path(out).write_bytes(head + dirs + b"".join(blobs))
