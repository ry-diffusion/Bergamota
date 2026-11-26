/* sysctlw NAME [INT]: write an integer sysctl, or read one (prints it in hex); the phone has no sysctl(8) */
#include <stdio.h>
#include <stdlib.h>
#include <sys/sysctl.h>

int
main(int argc, char **argv)
{
	if (argc != 2 && argc != 3) {
		fprintf(stderr, "usage: sysctlw NAME [INT]\n");
		return 2;
	}
	int v = argc == 3 ? atoi(argv[2]) : 0;
	size_t len = sizeof(v);
	int r = argc == 3 ? sysctlbyname(argv[1], NULL, NULL, &v, sizeof(v)) : sysctlbyname(argv[1], &v, &len, NULL, 0);
	if (r != 0) {
		perror(argv[1]);
		return 1;
	}
	if (argc == 2) {
		printf("%s: %#x\n", argv[1], (unsigned)v);
	}
	return 0;
}
