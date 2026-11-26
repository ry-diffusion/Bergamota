/*
 * bergamotad: root helper for the phone's Bergamota app (apps/BergamotaPhone). That app can only launch sandboxed here (an
 * ad-hoc app with no-sandbox/no-container entitlements is not started by SpringBoard), so it cannot write the performance
 * switches in /private/var/tmp, restart SpringBoard or reboot. It asks this daemon instead, over TCP on 127.0.0.1:7307 only,
 * one request line per connection, one reply line (or several for "get"):
 *   get                       every switch file present: NAME=HEX-OF-CONTENTS, then "end"
 *   set NAME HEX              write the file (HEX-encoded contents, up to 256 bytes)
 *   rm NAME                   remove the file
 *   respring                  SIGTERM to SpringBoard (backboardd keeps running)
 *   reboot                    /usr/local/bin/sweetreboot normal
 *   lpm                       "lpm STATE extreme X": iOS Low Power Mode (notify state) and kern.sweet_gpu_extreme
 * It also keeps the GPU's Extreme Performance Mode (kernel 1035, levels above 710 MHz) in step with iOS Low Power Mode:
 * on normally, off while Low Power Mode is on (notify "com.apple.system.lowpowermode", polled every 2 s).
 * NAME must be m2v-<lowercase letters, digits, dashes> or bgmtl-bb.env; files live in /private/var/tmp only.
 * Any local process can use it (a test phone): it binds the loopback address and nothing else.
 * Started by launchd: /System/Library/LaunchDaemons/org.sweet.bergamotad.plist. arm64e + pacstrip (tools/sweetnet/build.sh).
 */
#include <arpa/inet.h>
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <notify.h>
#include <pthread.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/sysctl.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;
#define PORT 7307
#define DIR "/private/var/tmp/"

static int
name_ok(const char *n)
{
	if (!strcmp(n, "bgmtl-bb.env"))
		return 1;
	if (strncmp(n, "m2v-", 4) || !n[4] || strlen(n) > 48)
		return 0;
	for (const char *p = n + 4; *p; p++)
		if (!(islower((unsigned char)*p) || isdigit((unsigned char)*p) || *p == '-'))
			return 0;
	return 1;
}

static void
reply(int fd, const char *s)
{
	(void)write(fd, s, strlen(s));
}

static void
get(int fd)
{
	static const char *names[] = { "m2v-sbkeepicons", "m2v-blurcap", "bgmtl-bb.env", "m2v-batch", "m2v-defer",
		"m2v-async-commit", "m2v-bb-nowait", "m2v-cut", "m2v-midcut", "m2v-hud", "m2v-fps", "m2v-telemetry",
		"m2v-cpumeter", NULL };
	for (int i = 0; names[i]; i++) {
		char path[128], buf[256], line[600];
		snprintf(path, sizeof(path), DIR "%s", names[i]);
		int f = open(path, O_RDONLY);
		if (f < 0)
			continue;
		ssize_t n = read(f, buf, sizeof(buf));
		close(f);
		int k = snprintf(line, sizeof(line), "%s=", names[i]);
		for (ssize_t j = 0; j < n && k < (int)sizeof(line) - 4; j++)
			k += snprintf(line + k, sizeof(line) - k, "%02x", (unsigned char)buf[j]);
		snprintf(line + k, sizeof(line) - k, "\n");
		reply(fd, line);
	}
	reply(fd, "end\n");
}

static int
set(const char *name, const char *hex)
{
	unsigned char buf[256];
	size_t n = strlen(hex);
	if (n % 2 || n / 2 > sizeof(buf))
		return -1;
	for (size_t i = 0; i < n / 2; i++) {
		unsigned v;
		if (sscanf(hex + 2 * i, "%2x", &v) != 1)
			return -1;
		buf[i] = (unsigned char)v;
	}
	char path[128];
	snprintf(path, sizeof(path), DIR "%s", name);
	int f = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW, 0644);
	if (f < 0)
		return -1;
	int ok = write(f, buf, n / 2) == (ssize_t)(n / 2);
	close(f);
	return ok ? 0 : -1;
}

static int
respring(void)
{
	int mib[3] = { CTL_KERN, KERN_PROC, KERN_PROC_ALL };
	size_t size = 0;
	if (sysctl(mib, 3, NULL, &size, NULL, 0))
		return -1;
	size += 64 * sizeof(struct kinfo_proc);
	struct kinfo_proc *p = malloc(size);
	if (!p || sysctl(mib, 3, p, &size, NULL, 0)) {
		free(p);
		return -1;
	}
	int found = -1;
	for (size_t i = 0; i < size / sizeof(*p); i++)
		if (!strcmp(p[i].kp_proc.p_comm, "SpringBoard")) {
			found = kill(p[i].kp_proc.p_pid, SIGTERM);
			break;
		}
	free(p);
	return found;
}

static int
reboot_normal(void)
{
	pid_t pid;
	char *argv[] = { "/usr/local/bin/sweetreboot", "normal", NULL };
	if (posix_spawn(&pid, argv[0], NULL, NULL, argv, environ))
		return -1;
	int st;
	waitpid(pid, &st, 0);
	return st == 0 ? 0 : -1;
}

static int lpm_token = -1;

static int
lpm_state(void)
{
	uint64_t st = 0;
	if (lpm_token < 0 || notify_get_state(lpm_token, &st) != NOTIFY_STATUS_OK)
		return -1;
	return st != 0;
}

/* Extreme Performance Mode follows Low Power Mode: written on every change and once at start (a kernel without 1035 ignores it). */
static void *
lpm_watch(void *arg)
{
	(void)arg;
	int last = -2;
	for (;;) {
		int lpm = lpm_state();
		if (lpm >= 0 && lpm != last) {
			int extreme = !lpm;
			int r = sysctlbyname("kern.sweet_gpu_extreme", NULL, NULL, &extreme, sizeof(extreme));
			fprintf(stderr, "bergamotad: Low Power Mode %s -> GPU Extreme Performance Mode %s%s\n", lpm ? "on" : "off",
			    extreme ? "on" : "off", r ? " (sysctl failed)" : "");
			last = lpm;
		}
		sleep(2);
	}
	return NULL;
}

int
main(void)
{
	pthread_t t;
	if (notify_register_check("com.apple.system.lowpowermode", &lpm_token) != NOTIFY_STATUS_OK)
		lpm_token = -1;
	pthread_create(&t, NULL, lpm_watch, NULL);
	signal(SIGPIPE, SIG_IGN);
	int s = socket(AF_INET, SOCK_STREAM, 0), one = 1;
	setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
	struct sockaddr_in a = { .sin_len = sizeof(a), .sin_family = AF_INET, .sin_port = htons(PORT) };
	a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	if (s < 0 || bind(s, (struct sockaddr *)&a, sizeof(a)) || listen(s, 4)) {
		perror("bergamotad");
		return 1;
	}
	for (;;) {
		int c = accept(s, NULL, NULL);
		if (c < 0)
			continue;
		struct timeval tv = { 2, 0 };
		setsockopt(c, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
		char line[700] = { 0 };
		ssize_t n = read(c, line, sizeof(line) - 1);
		if (n > 0) {
			line[strcspn(line, "\r\n")] = 0;
			char *cmd = strtok(line, " "), *name = strtok(NULL, " "), *arg = strtok(NULL, " ");
			if (!cmd)
				reply(c, "err empty\n");
			else if (!strcmp(cmd, "get"))
				get(c);
			else if (!strcmp(cmd, "lpm")) {
				int ex = -1;
				size_t len = sizeof(ex);
				char out[64];
				if (sysctlbyname("kern.sweet_gpu_extreme", &ex, &len, NULL, 0))
					ex = -1;
				snprintf(out, sizeof(out), "lpm %d extreme %d\n", lpm_state(), ex);
				reply(c, out);
			}
			else if (!strcmp(cmd, "set") && name && name_ok(name))
				reply(c, set(name, arg ? arg : "") ? "err write\n" : "ok\n");
			else if (!strcmp(cmd, "rm") && name && name_ok(name)) {
				char path[128];
				snprintf(path, sizeof(path), DIR "%s", name);
				reply(c, unlink(path) && errno != ENOENT ? "err unlink\n" : "ok\n");
			} else if (!strcmp(cmd, "respring"))
				reply(c, respring() ? "err no SpringBoard\n" : "ok\n");
			else if (!strcmp(cmd, "reboot")) {
				reply(c, "ok\n");
				close(c);
				reboot_normal();
				continue;
			} else
				reply(c, "err unknown request\n");
		}
		close(c);
	}
}
