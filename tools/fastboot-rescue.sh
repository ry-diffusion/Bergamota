#!/bin/bash
# Unattended fallback: when the phone sits in Qualcomm ABL fastboot, `fastboot reboot` it so ABL boots the boot
# partition (iOS) again. For overnight runs after a panic/watchdog reset that still lands in fastboot (kernel 1020
# makes panics and hangs reset into iOS by themselves; this catches the rest).
#   tools/fastboot-rescue.sh            loop forever (run it with nohup, or in a Terminal tab)
# Rules:
#   - only ABL fastboot: Bergamota fastboot (serial bergamota*) is left alone;
#   - the phone must stay in fastboot for GRACE s (default 90) first, so a person or the Bergamota queue can act;
#   - nothing while the Bergamota app runs with its queue armed (it is about to `fastboot boot` something);
#   - at most MAX reboots (default 3) per WINDOW s (default 3600): a boot loop does not spin forever.
# Env: POLL (10 s), GRACE, MAX, WINDOW, LOG (logs/fastboot-rescue.log), FASTBOOT (fastboot binary; tests stub it),
#      STATUS (Bergamota status.json), ONCE=1 (one poll, for tests).
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
POLL=${POLL:-10} GRACE=${GRACE:-90} MAX=${MAX:-3} WINDOW=${WINDOW:-3600}
LOG=${LOG:-$ROOT/logs/fastboot-rescue.log}
FASTBOOT=${FASTBOOT:-fastboot}
STATUS=${STATUS:-$HOME/Library/Application Support/Bergamota/status.json}
STATE=${STATE:-$ROOT/logs/.fastboot-rescue.state}   # "since" timestamp, then one line per reboot sent
mkdir -p "$(dirname "$LOG")"
log() { echo "$(date -u +%FT%TZ) $*" | tee -a "$LOG"; }

queue_armed() {
	pgrep -xq Bergamota || return 1
	python3 -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1]))["queue"]["armed"] else 1)' "$STATUS" 2>/dev/null
}

poll() {
	local now serial since recent
	now=$(date +%s)
	serial=$("$FASTBOOT" devices 2>/dev/null | awk '$2 == "fastboot" && $1 !~ /^bergamota/ { print $1; exit }')
	since=$(sed -n 1p "$STATE" 2>/dev/null)
	if [ -z "$serial" ]; then
		[ -n "$since" ] && { sed -i '' 1s/.*/0/ "$STATE"; }
		return
	fi
	if [ -z "$since" ] || [ "$since" = 0 ]; then
		log "ABL fastboot seen ($serial), waiting ${GRACE}s"
		if [ -f "$STATE" ]; then sed -i '' "1s/.*/$now/" "$STATE"; else echo "$now" > "$STATE"; fi
		return
	fi
	[ $((now - since)) -ge "$GRACE" ] || return
	if queue_armed; then
		log "Bergamota queue armed: leaving fastboot to it"
		return
	fi
	recent=$(sed 1d "$STATE" | awk -v t=$((now - WINDOW)) 'NF == 1 && $1 > t' | wc -l | tr -d ' ')
	if [ "$recent" -ge "$MAX" ]; then
		sed 1d "$STATE" | awk -v t=$((now - WINDOW)) '$2 == "limit" && $1 > t { f = 1 } END { exit !f }' ||
			{ log "rate limit: $MAX reboots in ${WINDOW}s, phone stays in fastboot"; echo "$now limit" >> "$STATE"; }
		return
	fi
	log "fastboot reboot ($serial, in fastboot $((now - since))s, $((recent + 1))/$MAX this window)"
	"$FASTBOOT" -s "$serial" reboot >> "$LOG" 2>&1 || log "fastboot reboot failed"
	sed -i '' 1s/.*/0/ "$STATE"
	echo "$now" >> "$STATE"
}

if [ "${ONCE:-0}" = 1 ]; then poll; exit 0; fi
log "started (poll ${POLL}s, grace ${GRACE}s, max $MAX per ${WINDOW}s)"
while :; do poll; sleep "$POLL"; done
