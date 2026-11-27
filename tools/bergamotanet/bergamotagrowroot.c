/*
 * bergamotagrowroot [--apply] [MIB]: grow the mounted iOS root (HFS+ on ufsroot0 = the whole userdata partition) with
 * the HFSIOC_RESIZE_VOLUME fsctl (hfs_extendfs in bsd/bergamota_hfs). Dry run unless --apply.
 * The block device reports the WHOLE partition, but its last blocks hold the persistent console log ring (patch 0047,
 * 4 MiB + header) and the UFS self-test area before it: the volume must end before them, so the target is capped at
 * USERDATA_MIB - RESERVE_MIB (Poco F1 userdata: 54791 MiB, kernel line "BERGAMOTA UFS: "userdata" ... 54791 MiB").
 * Default target = that cap. Back up first; the user confirms before --apply (it rewrites the volume header and bitmap).
 */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mount.h>
int fsctl(const char *, unsigned long, void *, unsigned int);   /* libSystem; no iOS SDK header */

#define USERDATA_MIB	54791ull
#define RESERVE_MIB	64ull	/* log ring (4 MiB + 4 KiB), self-test area, margin */
#define HFSIOC_RESIZE_VOLUME	_IOW('h', 2, u_int64_t)

int
main(int argc, char **argv)
{
	int apply = argc > 1 && strcmp(argv[1], "--apply") == 0;
	const char *arg = argc > 1 + apply ? argv[1 + apply] : NULL;
	uint64_t cap = (USERDATA_MIB - RESERVE_MIB) << 20;
	uint64_t want = arg ? strtoull(arg, NULL, 10) << 20 : cap;
	struct statfs st;
	if (statfs("/", &st) != 0) {
		perror("statfs /");
		return 1;
	}
	uint64_t cur = (uint64_t)st.f_blocks * st.f_bsize;
	printf("root: %s on %s, %llu MiB (%llu MiB free), block %u\n", st.f_fstypename, st.f_mntfromname,
	    cur >> 20, ((uint64_t)st.f_bavail * st.f_bsize) >> 20, st.f_bsize);
	printf("target: %llu MiB (cap %llu MiB = userdata %llu - reserve %llu)\n", want >> 20, cap >> 20, USERDATA_MIB, RESERVE_MIB);
	if (strcmp(st.f_fstypename, "hfs") != 0 || strcmp(st.f_mntfromname, "/dev/ufsroot0") != 0) {
		fprintf(stderr, "not the HFS root on /dev/ufsroot0: refusing\n");
		return 1;
	}
	if (want > cap || want <= cur) {
		fprintf(stderr, "target must be above the current size and at most the cap: refusing\n");
		return 1;
	}
	want -= want % st.f_bsize;
	if (!apply) {
		printf("dry run (add --apply to resize)\n");
		return 0;
	}
	if (fsctl("/", HFSIOC_RESIZE_VOLUME, &want, 0) != 0) {
		fprintf(stderr, "HFSIOC_RESIZE_VOLUME: %s\n", strerror(errno));
		return 1;
	}
	statfs("/", &st);
	printf("resized: %llu MiB (%llu MiB free)\n", ((uint64_t)st.f_blocks * st.f_bsize) >> 20,
	    ((uint64_t)st.f_bavail * st.f_bsize) >> 20);
	return 0;
}
