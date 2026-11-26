/* sysctlw NAME INT: write an integer sysctl (the phone has no sysctl(8)); e.g. kern.sweet_ddr_max 1 (kernel 1032) */
#include <stdio.h>
#include <stdlib.h>
#include <sys/sysctl.h>

int
main(int argc, char **argv)
{
	if (argc != 3) {
		fprintf(stderr, "usage: sysctlw NAME INT\n");
		return 2;
	}
	int v = atoi(argv[2]);
	if (sysctlbyname(argv[1], NULL, NULL, &v, sizeof(v)) != 0) {
		perror(argv[1]);
		return 1;
	}
	return 0;
}
