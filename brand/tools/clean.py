"""Strip Inkscape noise: keep only fill in style, drop ids/defs/namespaces."""
import re
import sys
from pathlib import Path

for p in map(Path, sys.argv[1:]):
    t = p.read_text()
    t = re.sub(r"<\?xml[^>]*\?>\s*", "", t)
    t = re.sub(r'style="[^"]*?fill:(#[0-9a-fA-F]{3,6})[^"]*"', r'fill="\1"', t)
    t = re.sub(r'\s(id|version|xmlns:svg|inkscape:\w+|sodipodi:\w+)="[^"]*"', "", t)
    t = re.sub(r'\sstyle="[^"]*"', "", t)  # no fill in style -> default black
    t = re.sub(r"<defs\s*/>", "", t)
    t = re.sub(r"\s+", " ", t).replace("> <", "><").strip()
    p.write_text(t + "\n")
