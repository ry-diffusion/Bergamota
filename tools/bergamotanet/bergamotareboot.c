/*
 * bergamotareboot fastboot|recovery|bergamota|normal: reboot the Poco F1 into that mode.
 * bergamotareboot poweroff: reboot(2) with RB_HALT; kernel patch 1036 turns the phone off (PSCI SYSTEM_OFF).
 *
 * Sets the root-only sysctl kern.bergamota_reboot_mode (kernel patch 0137), syncs
 * and calls the reboot(2) syscall (RB_AUTOBOOT), which reaches the kernel's
 * PE_halt_restart -> BergamotaPlatformExpert::haltRestart -> PS_HOLD reset after
 * writing the reason to the PM8998 PON block (and IMEM). A plain root reboot()
 * is what launchd/shutdown use underneath; no entitlement is needed beyond
 * root (the tool is also ad-hoc signed like the others, see build.sh).
 *
 * Needs a kernel with 0137: without the sysctl it refuses to reboot
 * (a reboot would otherwise just land in Android, or hang on older kernels).
 * Options: -n  set the mode only, do not reboot.
 * test-panic / test-hang (kernel patch 1020, kern.bergamota_reboot_test): a deliberate
 * panic, or stopped watchdog pets + a spinning CPU, to test the unattended recovery.
 */
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <sys/sysctl.h>
#include <unistd.h>

#define RB_AUTOBOOT_VALUE 0
#define RB_HALT_VALUE 0x08

int
main(int argc, char **argv)
{
	int noreboot = 0;
	const char *want = NULL;
	for (int i = 1; i < argc; i++) {
		if (!strcmp(argv[i], "-n")) {
			noreboot = 1;
		} else {
			want = argv[i];
		}
	}
	if (want && (!strcmp(want, "test-panic") || !strcmp(want, "test-hang"))) {
		int v = !strcmp(want, "test-panic") ? 1 : 2;
		sync();
		if (sysctlbyname("kern.bergamota_reboot_test", NULL, NULL, &v, sizeof(v)) != 0) {
			perror("bergamotareboot: sysctl kern.bergamota_reboot_test (kernel without patch 1020, or bergamotawdt off for test-hang?)");
			return 1;
		}
		return 0;	/* not reached for 1; 2 never returns either */
	}
	const char *mode = NULL;
	if (want && (!strcmp(want, "fastboot") || !strcmp(want, "bootloader"))) {
		mode = "bootloader";
	} else if (want && !strcmp(want, "recovery")) {
		mode = "recovery";
	} else if (want && !strcmp(want, "bergamota")) {
		mode = "bergamota";	/* kernel patch 0140: IMEM 'BERG', shim enters Bergamota fastboot */
	} else if (want && !strcmp(want, "normal")) {
		mode = "normal";
	} else if (want && (!strcmp(want, "poweroff") || !strcmp(want, "off"))) {
		mode = "poweroff";
	}
	if (!mode) {
		fprintf(stderr, "usage: bergamotareboot [-n] fastboot|recovery|bergamota|normal|poweroff | test-panic|test-hang\n");
		return 2;
	}
	if (geteuid() != 0) {
		fprintf(stderr, "bergamotareboot: must run as root\n");
		return 1;
	}
	int howto = RB_AUTOBOOT_VALUE;
	if (!strcmp(mode, "poweroff")) {
		howto = RB_HALT_VALUE;	/* no reboot mode; the kernel needs 1036, older ones halt with the phone on */
		goto go;
	}
	if (sysctlbyname("kern.bergamota_reboot_mode", NULL, NULL, (void *)mode, strlen(mode) + 1) != 0) {
		perror("bergamotareboot: sysctl kern.bergamota_reboot_mode (kernel without patch 0137, or 0140 for bergamota?)");
		return 1;
	}
	char got[32] = "";
	size_t len = sizeof(got);
	if (sysctlbyname("kern.bergamota_reboot_mode", got, &len, NULL, 0) != 0 || strcmp(got, mode) != 0) {
		fprintf(stderr, "bergamotareboot: mode readback '%s', expected '%s'\n", got, mode);
		return 1;
	}
	printf("bergamotareboot: next reboot mode %s\n", got);
go:
	if (noreboot) {
		return 0;
	}
	fflush(stdout);
	sync();
	sleep(1);
	sync();
	int (*reboot_fn)(int, ...) = (int (*)(int, ...))dlsym(RTLD_DEFAULT, "reboot");
	if (!reboot_fn) {
		fprintf(stderr, "bergamotareboot: reboot not found\n");
		return 1;
	}
	reboot_fn(howto);
	perror("bergamotareboot: reboot");
	return 1;
}
