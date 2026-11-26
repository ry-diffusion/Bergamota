#!/bin/sh
# Compile and run the kernel's berga code (from xnu/patches-11215/1037-*.patch) on the Mac, then check that
# berga-core decodes every frame it produced (core/tests/kernel_frames.rs reads core/tests/fixtures/kernel-frames.bin).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$here/../../..
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
python3 -I - "$repo"/xnu/patches-11215/1037-*.patch "$tmp/berga_block.c" <<'PY'
import sys
lines = open(sys.argv[1]).read().split("\n")
start = next(i for i, l in enumerate(lines) if l.startswith("+ * ---- berga: vendor interface 2")) - 1
out = []
for l in lines[start:]:
    if not l.startswith("+"):
        break
    out.append(l[1:])
open(sys.argv[2], "w").write("\n".join(out) + "\n")
PY
cc -std=gnu11 -Wall -Wextra -Wno-unused-function -Wno-unused-parameter -Wno-missing-field-initializers \
	-fsanitize=address,undefined -g -I"$tmp" -DBERGA_BLOCK='"berga_block.c"' "$here/harness.c" -o "$tmp/harness"
"$tmp/harness" "$here/../core/tests/fixtures/kernel-frames.bin"
cd "$here/.." && cargo test -q -p berga-core --test kernel_frames
