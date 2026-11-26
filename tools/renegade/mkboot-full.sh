#!/bin/bash
# Boot image for the FULL iOS root of the Renegade target (targets.py, RENEGADE_TARGET) on the userdata partition
# (rd=ufsroot0), no RAM disk.
#   tools/renegade/mkboot-full.sh [KERNEL] [ADT] [OUT.img]
# defaults (ios26): research/ios26/kernel/kernel.development.bergamota (xnu-12377), enxerto/out/beryllium-n104-2x.adt
#           (tools/mkadt.py --identity n104ap --display-scale 2 --points 414x896), research/ios26/enxerto-boot-ios26full.img
#           (ios18: research/ios18/kernel/kernel.development.bergamota (xnu-11215), enxerto/out/beryllium-hi-3x.adt)
# Test it with `fastboot boot OUT.img` (the phone must hold the image from tools/renegade/mkroot-full.sh
# flashed to userdata: docs/IOS18_PORT.md section 12). Nothing is flashed or booted here.
set -euo pipefail
cd "$(dirname "$0")/../.."
tg() { python3 -I tools/renegade/targets.py "$1"; }
NAME=$(tg name) WORK=$(tg work)
DEF_KERNEL=$WORK/kernel/kernel.development.bergamota
KERNEL=${1:-$DEF_KERNEL}
ADT=${2:-$(tg adt)}
OUT=${3:-$WORK/enxerto-boot-${NAME}full.img}
# RAM-disk cmdline of docs/IOS18_PORT.md section 9 with rd=md0 -> rd=ufsroot0 ufsrw=1 rootrw=1.
# ignition_level=1: dyld's libignition looks for the cache in /System/Library/Caches/com.apple.dyld only.
CMDLINE=${CMDLINE:-"debug=0xc serial=3 cpus=8 keepsyms=1 -noprogress rd=ufsroot0 unentitled_ios_sim_launch=1 ignition_level=1 wdt_test=no_user_checkin ufsrw=1 rootrw=1 launchd_unsecure_cache=1 wdt=-1 bergamotafb_noconsole=1 bergamotafb_hz=30 bergamotafb_vsync=1 bergamotafb_scale=1 bergamotafb_flip=1 bergamotausb=2 bergamotabattery=1 bergamotafb_hw=1 bergamotaarm64e=1 launchd_disable_all_timeouts=1 bergamotagpu=8 bergamotagpuuc=1 bergamotagpuboost=7 bergamotaddrmax=1"}
mkdir -p enxerto/out
# Apple logo as Enxerto's last frame (docs: tools/mkapplelogo.py): read from the iOS root you built, never
# committed. APPLE_LOGO=0 skips it (the Bergamota screen stays), APPLE_LOGO_PNG=file uses an extracted PNG.
rm -f enxerto/applelogo.h
if [ "${APPLE_LOGO:-1}" != 0 ]; then
	ROOT_SIMG=${ROOT_SIMG:-$WORK/${NAME}-root-full.simg}
	[ -e "$ROOT_SIMG" ] || ROOT_SIMG=../ios-beryllium-$NAME/$WORK/${NAME}-root-full.simg
	if [ -n "${APPLE_LOGO_PNG:-}" ]; then LOGO_SRC=(--png "$APPLE_LOGO_PNG"); else LOGO_SRC=(--root "$ROOT_SIMG"); fi
	python3 tools/mkapplelogo.py "${LOGO_SRC[@]}" || echo "warning: no Apple logo (Bergamota screen stays)" >&2
fi
python3 tools/mkpayload.py "$KERNEL" "$ADT" enxerto/out/payload-${NAME}full.bin --cmdline "$CMDLINE"
BOARD=beryllium enxerto/build.sh "$PWD/enxerto/out/payload-${NAME}full.bin"
cp enxerto/out/enxerto-boot.img "$OUT"
python3 tools/unpack_bootv2.py "$OUT" enxerto/out/check-${NAME}full >/dev/null 2>&1 && cmp enxerto/out/check-${NAME}full/ramdisk enxerto/out/payload-${NAME}full.bin && echo "payload check: ramdisk == payload"
ls -l "$OUT"
echo "cmdline: $CMDLINE"
