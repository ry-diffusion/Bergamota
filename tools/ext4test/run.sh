#!/bin/bash
# Builds ext2/ext4 test images with mke2fs -d (Homebrew e2fsprogs) and an liblp-style super, runs the host test.
set -euo pipefail
cd "$(dirname "$0")"
W=$(mktemp -d /tmp/ext4test.XXXXXX)
cc -std=gnu11 -Wall -Wextra -O1 -g -o "$W/t" ext4_host_test.c
S=$W/src; mkdir -p $S/sensors/registry/registry $S/etc/sensors/config $S/deep/a/b/c
for i in $(seq 1 60); do echo "reg $i" > $S/sensors/registry/registry/bmi160_$i; done
for i in $(seq 1 300); do echo "{ \"cfg\": $i }" > $S/etc/sensors/config/long_file_name_for_hashing_$i.json; done
head -c 5300000 /dev/urandom > $S/big.bin            # double-indirect on ext2
head -c 100 /dev/urandom > $S/small.bin
: > $S/empty
python3 - "$S/sparse.bin" <<'PY'
import sys
f=open(sys.argv[1],'wb'); f.write(b'A'*100); f.seek(3000000); f.write(b'B'*50); f.close()
PY
ln -s etc/sensors $S/link_rel; ln -s /deep/a/b $S/link_abs
echo hello > $S/deep/a/b/c/leaf
for cfg in "ext4 4096 extents,64bit,flex_bg" "ext4 1024 ^64bit" "ext2 2048 ^dir_index" "ext2 4096 none"; do
  set -- $cfg
  img=$W/$1_$2.img; rm -f $img
  if [ "$3" = none ]; then mke2fs -q -t $1 -b $2 -d $S $img 24M; else mke2fs -q -t $1 -b $2 -O ^has_journal,$3 -d $S $img 24M 2>/dev/null || mke2fs -q -t $1 -b $2 -d $S $img 24M; fi
  echo "== $cfg"; "$W/t" $img $S | grep -v '^  '
done
# liblp: a super with one extent at sector 2048 holding the ext4 image
python3 - "$W" <<'PY'
import struct,sys
W=sys.argv[1]
fsimg=open(W+'/ext4_4096.img','rb').read()
max_meta=65536
part=b'vendor'.ljust(36,b'\0')+struct.pack('<IIII',0,0,1,0)
ext=struct.pack('<QIQI',len(fsimg)//512,0,2048,0)
hs=128
tables=part+ext
hdr=struct.pack('<IHHI',0x414c5030,10,0,hs)+b'\0'*32+struct.pack('<I',len(tables))+b'\0'*32
hdr+=struct.pack('<III',0,1,52)+struct.pack('<III',52,1,24)+struct.pack('<III',0,0,0)+struct.pack('<III',0,0,0)
assert len(hdr)==hs,len(hdr)
geo=struct.pack('<II',0x616c4467,52)+b'\0'*32+struct.pack('<III',max_meta,2,4096)
sup=bytearray(2048*512+len(fsimg))
sup[4096:4096+len(geo)]=geo
sup[8192:8192+len(geo)]=geo
sup[12288:12288+len(hdr)+len(tables)]=hdr+tables
sup[12288+max_meta:12288+max_meta+len(hdr)+len(tables)]=hdr+tables
sup[2048*512:]=fsimg
open(W+'/super.img','wb').write(sup)
PY
echo "== liblp super"; "$W/t" $W/super.img $S $W/super.img vendor | grep -v '^  '
rm -rf "$W"
