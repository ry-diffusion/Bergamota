#!/bin/sh
# Build and ad-hoc sign the on-device tools for iOS 16 -> research/bergamotanet/
# (bergamotanetcfg, cscheck, hcitest, bluetool-stub, bergamotareboot, bergamotadur = UFS durability test, kernel 0221; set TOOLS="hcitest" to build a subset; bergamotareboot = reboot-to-fastboot, kernel 0137)
# iOS 18: also research/bergamotanet/ios18/{bergamotauicache,bergamotafp.dylib} (arm64e + pacstrip --sign)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/../../research/bergamotanet
mkdir -p "$out"
for t in ${TOOLS:-bergamotanetcfg cscheck hcitest bluetool-stub bergamotareboot bergamotadur}; do
	ents=$here/entitlements.plist
	case $t in
	cscheck) frameworks="-framework CoreFoundation -framework Security" ;;
	hcitest|bergamotareboot|bergamotadur) frameworks="" ;;
	bluetool-stub) frameworks=""; ents=$here/bluetool.entitlements ;;
	*) frameworks="-framework CoreFoundation" ;;
	esac
	xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=16.0 -O2 -Wall \
		$frameworks "$here/$t.c" -o "$out/$t"
	codesign -s - -f --entitlements "$ents" "$out/$t"
	ls -l "$out/$t"
done
# LaunchServices register tool for sideloaded apps (tools/bergamota-install-ipa.sh)
xcrun -sdk iphoneos clang -arch arm64 -mios-version-min=16.0 -O2 -fobjc-arc \
	-framework Foundation "$here/bergamotauicache.m" -o "$out/bergamotauicache"
codesign -s - -f --entitlements "$here/uicache.entitlements" "$out/bergamotauicache"

# iOS 18 (Renegade): arm64e builds that the kernel without pointer authentication runs after
# `renegade strip --sign` (pacstrip). Output: research/bergamotanet/ios18/ (set IOS18=0 to skip)
if [ "${IOS18:-1}" = 1 ]; then
	out18=$out/ios18
	pacstrip=$here/../renegade/pacstrip.py
	mkdir -p "$out18"
	# bergamotaprefs: set/read a user default through cfprefsd (installed in /usr/local/bin by root_tweaks18.base())
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall -framework CoreFoundation \
		"$here/bergamotaprefs.c" -o "$out18/bergamotaprefs"
	codesign -s - -f --entitlements "$here/entitlements.plist" "$out18/bergamotaprefs"
	python3 -I "$pacstrip" --sign "$out18/bergamotaprefs"
	# bergamotadur: UFS/HFS durability test (kernel patch 0221)
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall "$here/bergamotadur.c" -o "$out18/bergamotadur"
	codesign -s - -f --entitlements "$here/entitlements.plist" "$out18/bergamotadur"
	python3 -I "$pacstrip" --sign "$out18/bergamotadur"
	# bergamotauicache: LaunchServices registration, run by the first-boot job (root_tweaks18.firstboot)
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -fobjc-arc \
		-framework Foundation "$here/bergamotauicache.m" -o "$out18/bergamotauicache"
	codesign -s - -f --entitlements "$here/uicache.entitlements" "$out18/bergamotauicache"
	python3 -I "$pacstrip" --sign "$out18/bergamotauicache"
	# bergamotafp.dylib: NSURLFileProtectionKey shim, linked into PosterBoard (root_tweaks18.posterboard)
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -fobjc-arc -dynamiclib \
		-install_name /usr/local/lib/bergamotafp.dylib -framework Foundation \
		"$here/bergamotafp.m" -o "$out18/bergamotafp.dylib"
	codesign -s - -f "$out18/bergamotafp.dylib"
	python3 -I "$pacstrip" --sign "$out18/bergamotafp.dylib"
	ls -l "$out18"
fi

# First-boot wallpaper/icon style and the mobile-domain runner (root_tweaks18.firstboot, iOS 18, arm64e + pacstrip)
if [ "${IOS18:-1}" = 1 ]; then
	out18=$out/ios18
	pacstrip=$here/../renegade/pacstrip.py
	b18() { # name source entitlements [extra clang args]
		n=$1; src=$2; ents=$3; shift 3
		xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -fobjc-arc -framework Foundation "$@" \
			"$here/$src" -o "$out18/$n"
		codesign -s - -f --entitlements "$here/$ents" "$out18/$n"
		python3 -I "$pacstrip" --sign "$out18/$n"
	}
	# bergamotametrics: Bergamota live metrics (docs/BERGAMOTA_METRICS.md), plain C, root_tweaks18.bergamotametrics
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall -framework CoreFoundation -framework IOKit \
		"$here/bergamotametrics.c" -o "$out18/bergamotametrics"
	codesign -s - -f --entitlements "$here/entitlements.plist" "$out18/bergamotametrics"
	python3 -I "$pacstrip" --sign "$out18/bergamotametrics"
	# test harness: Auto-Lock (ManagedConfiguration maxInactivity, "never") and SpringBoard's lock state (notify key)
	b18 autolock autolock.m autolock.entitlements
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall "$here/lockstate.c" -o "$out18/lockstate"
	codesign -s - -f "$out18/lockstate"
	python3 -I "$pacstrip" --sign "$out18/lockstate"
	# bergamotawifijoin: join a network through wifid (MobileWiFi), passphrase on stdin (tools/berga-wifi-test.sh)
	b18 bergamotawifijoin bergamotawifijoin.m wifijoin.entitlements
	# sblaunch: SpringBoard's launchd Program when it runs on Crimson (DYLD_INSERT_LIBRARIES, then exec; root_tweaks18.crimson)
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall "$here/sblaunch.c" -o "$out18/sblaunch"
	codesign -s - -f --entitlements "$here/entitlements.plist" "$out18/sblaunch"
	python3 -I "$pacstrip" --sign "$out18/sblaunch"
	b18 bergamotaposterset bergamotaposterset.m posterset.entitlements
	# a picture as the lock and home wallpaper through PosterBoard's Photos poster (tools/identity/mkwallpaper.sh)
	b18 bergamotawallpaper bergamotawallpaper.m posterset.entitlements
	# iOS 26: the Bergamota picture as the legacy wallpaper, drawn by PosterBoard's LegacyPoster (no HEIC, no Photos pipeline)
	b18 bergamotalegacywall bergamotalegacywall.m posterset.entitlements -framework UIKit
	# iOS 26: PosterBoard on the HFS root without data protection (file protection, purgeable, the empty StandBy role)
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -fobjc-arc -dynamiclib \
		-install_name /usr/local/lib/bergamotapb26.dylib -framework Foundation "$here/bergamotapb26.m" -o "$out18/bergamotapb26.dylib"
	codesign -s - -f "$out18/bergamotapb26.dylib"
	python3 -I "$pacstrip" --sign "$out18/bergamotapb26.dylib"
	b18 bergamotaiconstyle bergamotaiconstyle.m iconstyle.entitlements -framework UIKit
	b18 bergamotaopen bergamotaopen.m uicache.entitlements
	# remote testing over SSH: screenshot of the panel (PNG) and synthetic touch/buttons
	b18 bergamotashot bergamotashot.m touch.entitlements -framework CoreGraphics -framework ImageIO -framework IOSurface \
		-framework QuartzCore
	# a CPU profile of one process over SSH (running threads only, frame-pointer stacks, aggregated on the phone)
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall "$here/bergamotasample.c" -o "$out18/bergamotasample"
	codesign -s - -f --entitlements "$here/sample.entitlements" "$out18/bergamotasample"
	python3 -I "$pacstrip" --sign "$out18/bergamotasample"
	# root helper of the phone's Bergamota app (apps/BergamotaPhone): switches, respring, reboot on 127.0.0.1:7307
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall "$here/bergamotad.c" -o "$out18/bergamotad"
	codesign -s - -f --entitlements "$here/entitlements.plist" "$out18/bergamotad"
	python3 -I "$pacstrip" --sign "$out18/bergamotad"
	# Wi-Fi over SSH (iOS 26 stand-in, docs/WIFI_IOS26.md): Apple80211 info/power/scan without wifid or the UI
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall -framework CoreFoundation \
		"$here/bergamotawifi.c" -o "$out18/bergamotawifi"
	codesign -s - -f --entitlements "$here/wifi.entitlements" "$out18/bergamotawifi"
	python3 -I "$pacstrip" --sign "$out18/bergamotawifi"
	# the unified log over SSH (no `log` tool on iOS)
	b18 bergamotalog bergamotalog.m log.entitlements -framework OSLog
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall -framework CoreFoundation -framework IOKit \
		"$here/bergamotatouch.c" -o "$out18/bergamotatouch"
	codesign -s - -f --entitlements "$here/touch.entitlements" "$out18/bergamotatouch"
	python3 -I "$pacstrip" --sign "$out18/bergamotatouch"
	# BergamotaRun.app: runs /private/var/tmp/bergamotarun.sh as mobile in the user's launchd domain (an app started by FrontBoard)
	app=$out18/BergamotaRun.app
	rm -rf "$app"; mkdir -p "$app"
	cp "$here/bergamotarun-app/Info.plist" "$app/Info.plist"
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -fobjc-arc -framework UIKit -framework Foundation \
		"$here/bergamotarun-app/bergamotarun.m" -o "$app/BergamotaRun"
	codesign -s - -f --entitlements "$here/bergamotarun-app/run.entitlements" "$app"
	python3 -I "$pacstrip" --sign "$app/BergamotaRun"
	codesign -s - -f --entitlements "$here/bergamotarun-app/run.entitlements" "$app"      # seal the bundle again
	ls -l "$out18"
fi

# bergamotametrics (Bergamota live metrics, docs/BERGAMOTA_METRICS.md): iOS 18, arm64e, then `renegade strip --sign`
# (pacstrip) so the kernel without pointer authentication runs it. Output: research/bergamotanet/ios18/bergamotametrics
if [ "${IOS18:-1}" = 1 ]; then
	out18=$out/ios18
	mkdir -p "$out18"
	xcrun -sdk iphoneos clang -arch arm64e -mios-version-min=18.0 -O2 -Wall \
		-framework CoreFoundation -framework IOKit "$here/bergamotametrics.c" -o "$out18/bergamotametrics"
	codesign -s - -f --entitlements "$here/entitlements.plist" "$out18/bergamotametrics"
	"$here/../renegade/renegade" strip --sign "$out18/bergamotametrics"
	ls -l "$out18/bergamotametrics"
fi
