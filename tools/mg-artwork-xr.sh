#!/bin/bash
# mg-artwork-xr.sh: set MobileGestalt's cached artwork traits on the phone to the iPhone XR class (@2x, subtype 1792), matching
# the 828x1792 canvas (beryllium-hi-2x-xr ADT). The D22 (iPhone X) identity makes MobileGestalt answer @3x/2436, so UIKit finds no
# status bar layout for 414x896 @2x and draws a red "rdar:45025538" banner. Run once per root (the cache is rebuilt after a
# reflash or when it is deleted), then reboot. Keeps the phone's original as /var/root/MobileGestalt.plist.before-xr.
set -euo pipefail
PHONE=${PHONE:-192.168.7.2}
F=/private/var/containers/Shared/SystemGroup/systemgroup.com.apple.mobilegestaltcache/Library/Caches/com.apple.MobileGestalt.plist
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
scp -O -q root@"$PHONE":"$F" "$T/mg.plist"
K='CacheExtra.oPeik/9e8lQWMszEjbPzng'
plutil -replace "$K.ArtworkDeviceScaleFactor" -integer 2 "$T/mg.plist"
plutil -replace "$K.ArtworkDeviceSubType" -integer 1792 "$T/mg.plist"
plutil -replace "$K.ArtworkDeviceProductDescription" -string "iPhone XR" "$T/mg.plist"
plutil -lint "$T/mg.plist" >/dev/null
plutil -convert binary1 "$T/mg.plist"
scp -O -q "$T/mg.plist" root@"$PHONE":/var/root/MobileGestalt.plist.xr
ssh root@"$PHONE" "[ -e /var/root/MobileGestalt.plist.before-xr ] || cp -p $F /var/root/MobileGestalt.plist.before-xr; cp /var/root/MobileGestalt.plist.xr $F.tmp && chown mobile:nobody $F.tmp && chmod 644 $F.tmp && mv $F.tmp $F"
echo "artwork traits set to @2x/1792; reboot the phone"
