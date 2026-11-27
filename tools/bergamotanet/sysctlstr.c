/* sysctlstr NAME: print a string sysctl of any length (e.g. kern.bergamota_log_prev, the previous boot's console tail) */
#include <stdio.h>
#include <stdlib.h>
#include <sys/sysctl.h>

int
main(int argc, char **argv)
{
	if (argc != 2) {
		fprintf(stderr, "usage: sysctlstr NAME\n");
		return 2;
	}
	size_t len = 0;
	if (sysctlbyname(argv[1], NULL, &len, NULL, 0) != 0) {
		perror(argv[1]);
		return 1;
	}
	char *buf = malloc(len + 1);
	if (!buf || sysctlbyname(argv[1], buf, &len, NULL, 0) != 0) {
		perror(argv[1]);
		return 1;
	}
	buf[len] = 0;
	fputs(buf, stdout);
	return 0;
}
