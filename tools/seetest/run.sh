#!/bin/bash
# Host test of the SEE client's protobuf/float helpers: extracts them from the kernel source, compares known encodings.
set -euo pipefail
cd "$(dirname "$0")"
SRC=${1:-../../research/xnu-local/dxb/xnu/bsd/dev/beryllium_wlan.c}
W=$(mktemp -d /tmp/seetest.XXXXXX)
python3 -I - "$SRC" "$W/see_core.inc" <<'PY'
import sys
s=open(sys.argv[1]).read()
a=s.index('/* ---- integer float helpers */'); b=s.index('/* ---- QMI over QRTR to the SSC service */')
open(sys.argv[2],'w').write(s[a:b])
PY
cc -std=gnu11 -Wall -Wextra -I"$W" -o "$W/t" see_codec_test.c
"$W/t"
rm -rf "$W"
