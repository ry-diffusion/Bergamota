#!/bin/bash
# Self-check of tools/fastboot-rescue.sh with a stub fastboot: grace period, Bergamota fastboot ignored, rate limit.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
printf '#!/bin/sh\n[ "$1" = devices ] && cat "%s/dev" && exit 0\necho "$*" >> "%s/sent"\n' "$t" "$t" > "$t/fb"; chmod +x "$t/fb"
run() { ONCE=1 GRACE=0 MAX=2 WINDOW=3600 FASTBOOT=$t/fb LOG=$t/log STATE=$t/state STATUS=/nonexistent "$here/fastboot-rescue.sh" >/dev/null; }
sent() { [ -f "$t/sent" ] && wc -l < "$t/sent" | tr -d ' ' || echo 0; }
printf 'bergamota0001\tfastboot\n' > "$t/dev"; run; run; [ "$(sent)" = 0 ] || { echo "FAIL: rebooted Bergamota fastboot"; exit 1; }
printf 'abc123\tfastboot\n' > "$t/dev"
run; [ "$(sent)" = 0 ] || { echo "FAIL: no grace poll"; exit 1; }   # first sighting only starts the clock
run; [ "$(sent)" = 1 ] || { echo "FAIL: first reboot"; exit 1; }
run; run; [ "$(sent)" = 2 ] || { echo "FAIL: second reboot"; exit 1; }
run; run; run; run; [ "$(sent)" = 2 ] || { echo "FAIL: rate limit"; exit 1; }
[ "$(grep -c 'rate limit' "$t/log")" = 1 ] || { echo "FAIL: limit logged once"; exit 1; }
grep -q -- '-s abc123 reboot' "$t/sent" || { echo "FAIL: command"; exit 1; }
echo PASS
