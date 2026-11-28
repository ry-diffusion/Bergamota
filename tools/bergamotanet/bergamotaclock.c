/* bergamotaclock save|restore: the phone has no usable RTC, so every boot starts at 1969-12-31 until Wi-Fi brings network time.
 *   save     write the current time to /private/var/db/bergamota/clock (atomic), only if the clock is sane (not before the build time)
 *   restore  if the clock is earlier than max(saved time, build time), set it to that; never moves a sane clock backwards
 * Run at boot / every 10 minutes by the launchd jobs of root_tweaks18.bergamotaclock(). The build time is baked in at compile
 * time (tools/bergamotanet/build.sh: -DBUILD_TIME=<epoch>).
 * Design after clock_restore() in init.c of the Galaxy S20 port, Jesvi iOS (MIT). */
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <syslog.h>
#include <unistd.h>

#ifndef BUILD_TIME
#error "compile with -DBUILD_TIME=<epoch seconds> (tools/bergamotanet/build.sh)"
#endif
#ifndef DIR
#define DIR "/private/var/db/bergamota"
#endif
#define MARK DIR "/clock"

static void say(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void say(const char *fmt, ...)
{
	char b[200];
	va_list ap;
	va_start(ap, fmt);
	vsnprintf(b, sizeof b, fmt, ap);
	va_end(ap);
	printf("bergamotaclock: %s\n", b);
	syslog(LOG_NOTICE, "%s", b);
}

static long read_saved(void)
{
	char b[32] = "";
	char *end;
	long v;
	int fd = open(MARK, O_RDONLY);
	ssize_t n = fd < 0 ? -1 : read(fd, b, sizeof b - 1);
	if (fd >= 0) close(fd);
	if (n <= 0) return 0;
	v = strtol(b, &end, 10);
	return end == b || v < 0 ? 0 : v;   /* garbage or a torn file: ignored, the build time still applies */
}

static int save(void)
{
	struct timeval tv;
	char b[32], tmp[] = MARK ".tmp";
	int fd, n;

	gettimeofday(&tv, NULL);
	if (tv.tv_sec < BUILD_TIME) {
		say("save: clock %ld is before the build time %ld, not saved", (long)tv.tv_sec, (long)BUILD_TIME);
		return 0;
	}
	mkdir(DIR, 0755);
	n = snprintf(b, sizeof b, "%ld\n", (long)tv.tv_sec);
	fd = open(tmp, O_WRONLY | O_CREAT | O_TRUNC, 0644);
	if (fd < 0 || write(fd, b, n) != n || fsync(fd) != 0 || close(fd) != 0 || rename(tmp, MARK) != 0) {
		say("save: %s", strerror(errno));
		return 1;
	}
	return 0;
}

static int restore(void)
{
	struct timeval tv;
	long saved = read_saved(), target = saved > BUILD_TIME ? saved : BUILD_TIME;

	gettimeofday(&tv, NULL);
	if (tv.tv_sec >= target) {
		say("restore: clock %ld already at or after %ld, left alone", (long)tv.tv_sec, target);
		return 0;
	}
	tv.tv_sec = target;
	tv.tv_usec = 0;
	if (settimeofday(&tv, NULL) != 0) {
		say("restore: settimeofday(%ld): %s", target, strerror(errno));
		return 1;
	}
	say("restore: clock set to %ld (%s); network sync pending", target, target == saved ? "saved time" : "image build time");
	return 0;
}

int main(int argc, char **argv)
{
	openlog("bergamotaclock", LOG_PID, LOG_DAEMON);
	if (argc == 2 && !strcmp(argv[1], "save")) return save();
	if (argc == 2 && !strcmp(argv[1], "restore")) return restore();
	fprintf(stderr, "usage: bergamotaclock save|restore\n");
	return 2;
}
