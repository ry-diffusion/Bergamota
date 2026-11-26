#!/bin/sh
# Build bergad for the phone (plain arm64: runs without pacstrip) and ad-hoc sign it with the phone tools' entitlements.
#   tools/berga/phone/build.sh  ->  tools/berga/target/aarch64-apple-ios/release/bergad
# Install (iOS up, SSH; then the next boot starts it, or launchctl load it now):
#   scp -O target/aarch64-apple-ios/release/bergad root@192.168.7.2:/usr/local/bin/bergad
#   scp -O phone/launchd/org.bergamota.bergad.plist root@192.168.7.2:/System/Library/LaunchDaemons/
#   ssh root@192.168.7.2 launchctl load /System/Library/LaunchDaemons/org.bergamota.bergad.plist
# Needs: rustup target add aarch64-apple-ios
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/.."
cargo build -p bergad --release --target aarch64-apple-ios
bin=target/aarch64-apple-ios/release/bergad
codesign -f -s - --entitlements "$here/../../bergamotanet/entitlements.plist" "$bin"
ls -l "$bin"
