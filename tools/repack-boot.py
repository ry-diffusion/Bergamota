#!/usr/bin/env python3
"""Rebuild an Enxerto boot.img (header v1) with another xnu kernel and/or more boot-args, keeping its Enxerto
blob, ADT and cmdline otherwise: the recipe the Wi-Fi/GPU images were made with.

  repack-boot.py IN.img OUT.img [--kernel KERNEL] [--add-args "a=1 b=2"]

With neither option it re-creates IN.img byte for byte from its own pieces when given the kernel it was built
with (checked on boot55 with kernel-1130). Needs mkbootimg and tools/mkpayload.py.
"""
import argparse, os, struct, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mkpayload  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument('inp'); ap.add_argument('out')
ap.add_argument('--kernel', required=True, help='xnu kernel Mach-O (kernel.development.sweet)')
ap.add_argument('--add-args', default='')
a = ap.parse_args()

d = open(a.inp, 'rb').read()
assert d[:8] == b'ANDROID!', 'not a boot.img'
ksize, kaddr, rsize, raddr, _, _, tags, page, hver, osver = struct.unpack_from('<10I', d, 8)
assert hver == 1, f'header v{hver}, expected 1'
cmd = d[64:64 + 512].split(b'\0')[0].decode()
pg = lambda n: (n + page - 1) // page * page
kblob = d[page:page + ksize]
payload = d[page + pg(ksize):page + pg(ksize) + rsize]
assert payload[:8] == b'SWEETXNU', 'ramdisk is not an xnu payload'
ver, = struct.unpack_from('<I', payload, 8)
adt_off, adt_size = struct.unpack_from('<2Q', payload, 0x20)
args = payload[mkpayload.CMDLINE:mkpayload.CMDLINE + mkpayload.BOOT_LINE_LENGTH].split(b'\0')[0].decode()
if a.add_args:
    args = f'{args} {a.add_args}'
assert len(args) < mkpayload.BOOT_LINE_LENGTH, f'boot-args too long ({len(args)})'
osv = osver >> 11
patch = osver & 0x7ff

with tempfile.TemporaryDirectory() as t:
    adt = os.path.join(t, 'adt'); kb = os.path.join(t, 'k'); pl = os.path.join(t, 'p')
    open(adt, 'wb').write(payload[adt_off:adt_off + adt_size])
    open(kb, 'wb').write(kblob)
    # the ADT already carries the img4 nodes mkpayload added the first time
    blob, _ = mkpayload.build(a.kernel, adt, args, None, ver == 2, False)
    open(pl, 'wb').write(blob)
    subprocess.run(['mkbootimg', '--header_version', '1', '--base', '0', '--kernel_offset', hex(kaddr),
                    '--ramdisk_offset', hex(raddr), '--tags_offset', hex(tags), '--second_offset', '0',
                    '--pagesize', str(page),
                    '--os_version', f'{osv >> 14}.{(osv >> 7) & 0x7f}.{osv & 0x7f}',
                    '--os_patch_level', f'{2000 + (patch >> 4)}-{patch & 0xf:02d}',
                    '--kernel', kb, '--ramdisk', pl, '--cmdline', cmd, '-o', a.out], check=True)
print(f'{a.out}: boot-args ({len(args)} bytes): {args}')
