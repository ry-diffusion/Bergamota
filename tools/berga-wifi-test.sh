#!/bin/bash
# berga-wifi-test.sh [SSID]: join the phone (iOS 18, Wi-Fi boot image) to a Wi-Fi network through wifid and test it.
# Run it yourself on the Mac: it asks for the passphrase (not echoed, not stored) and pipes it to
# bergamotawifijoin on the phone over the USB SSH link; then it checks the address, pings the router and
# measures a download from this Mac over the Wi-Fi network (the Mac must be on the same network).
#   tools/berga-wifi-test.sh "MyNetwork"
# Needs: the phone up on a boot image with bergamotawlan=5 bergamota80211=3 and the /arm-io/wlan ADT node,
# bergamotawifijoin built (tools/bergamotanet/build.sh or the BIN= path below), python3 on the Mac.
set -euo pipefail
PHONE=${PHONE:-192.168.7.2}
KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}
BIN=${BIN:-$(dirname "$0")/../research/bergamotanet/ios18/bergamotawifijoin}
SSHO=(-o BatchMode=yes -o ConnectTimeout=8 -o UserKnownHostsFile="$KH")
P() { ssh "${SSHO[@]}" root@"$PHONE" "$@"; }

ssid=${1:-}
[ -n "$ssid" ] || read -r -p "Wi-Fi network name: " ssid
read -r -s -p "Passphrase for $ssid (empty for an open network): " pass; echo

[ -f "$BIN" ] || { echo "no $BIN (build it: tools/bergamotanet/build.sh)" >&2; exit 1; }
scp -O -q "${SSHO[@]}" "$BIN" root@"$PHONE":/var/root/bergamotawifijoin.tmp
P 'chmod 755 /var/root/bergamotawifijoin.tmp && mv /var/root/bergamotawifijoin.tmp /var/root/bergamotawifijoin'

echo "== join"
printf '%s\n' "$pass" | P "/var/root/bergamotawifijoin '$ssid' 40" || { echo "join failed"; exit 1; }
unset pass

echo "== phone side"
P 'ifconfig en0 | grep -E "inet |status"; netstat -rn -f inet 2>/dev/null | grep -E "^default" | head -2'
router=$(P 'netstat -rn -f inet 2>/dev/null | grep -E "^default.*en0" | head -1' | awk '{print $2}')
[ -n "$router" ] && P "ping -c 4 $router 2>&1 | tail -2" || echo "(no default route via en0)"

echo "== throughput: this Mac -> phone over Wi-Fi"
macip=$(ipconfig getifaddr en0 || true)
[ -n "$macip" ] || { echo "this Mac has no Wi-Fi address on en0: skipped"; exit 0; }
port=5299; mb=40
python3 - "$port" "$mb" <<'PY' &
import socket, sys
port, mb = int(sys.argv[1]), int(sys.argv[2])
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(('0.0.0.0', port)); s.listen(1)
s.settimeout(60)
c, _ = s.accept(); buf = b'\0' * 65536
for _ in range(mb * 16): c.sendall(buf)
c.close()
PY
sleep 1
P "t0=\$(date +%s); nc $macip $port | wc -c; t1=\$(date +%s); echo \"seconds \$((t1-t0))\"" | tr '\n' ' '; echo
wait || true
echo "(MB/s ~= $mb / seconds; the phone reaches $macip through its Wi-Fi default route)"
