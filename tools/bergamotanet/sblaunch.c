/*
 * sblaunch: SpringBoard's launchd Program on iOS 18 when it runs on Crimson. Sets DYLD_INSERT_LIBRARIES to the Crimson
 * inject and execs the real SpringBoard (same pid, so launchd's job and Mach-service check-in are unchanged).
 *
 * Why: iOS 18's SpringBoard has 16 bytes of load-command slack (the inject's LC_LOAD_DYLIB needs 72), and launchd drops
 * DYLD_* from a job's EnvironmentVariables, while dyld honours them from execve (kernel 0131 allows DYLD_* for arm64e
 * with bergamotaarm64e=1). The inject itself still needs /private/var/tmp/crimson.arm and crimson.app.com.apple.springboard;
 * /private/var/tmp/sblaunch.off makes this a plain exec (kill switch without touching launchd.plist).
 */
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#define SPRINGBOARD "/System/Library/CoreServices/SpringBoard.app/SpringBoard"
#define INJECT "/usr/local/lib/crimson/crimson-inject.dylib"

int
main(int argc, char **argv)
{
	(void)argc;
	if (access("/private/var/tmp/sblaunch.off", F_OK) != 0 && access(INJECT, R_OK) == 0)
		setenv("DYLD_INSERT_LIBRARIES", INJECT, 1);
	argv[0] = SPRINGBOARD;
	execv(SPRINGBOARD, argv);
	perror("sblaunch: execv " SPRINGBOARD);
	return 127;
}
