/* Host test for bergamota_ext4.c: ext4_host_test IMAGE SRCDIR [SUPER NAME]
 * Compares every file under SRCDIR with the same path read through the reader from IMAGE (or from the logical
 * partition NAME inside the liblp image SUPER), and lists the root. Exit 0 = all equal. */
#define E4_HOST_TEST 1
#include "bergamota_ext4.c"
#include <dirent.h>
#include <sys/stat.h>

static FILE *img;

static int
rd(void *ctx, uint64_t off, void *dst, uint64_t len)
{
	(void)ctx;
	if (fseeko(img, (off_t)off, SEEK_SET) || fread(dst, 1, len, img) != len) {
		return EIO;
	}
	return 0;
}

static struct e4_fs fs;
static unsigned nfiles, nbad;

static int
list_cb(const char *n, unsigned l, uint32_t ino, unsigned t, void *c)
{
	(void)c;
	printf("  %.*s ino %u type %u\n", (int)l, n, ino, t);
	return 0;
}

static void
walk(const char *host, const char *guest)
{
	DIR *d = opendir(host);
	struct dirent *e;
	while (d && (e = readdir(d))) {
		if (!strcmp(e->d_name, ".") || !strcmp(e->d_name, "..")) {
			continue;
		}
		char h[1024], g[512];
		snprintf(h, sizeof(h), "%s/%s", host, e->d_name);
		snprintf(g, sizeof(g), "%s/%s", guest, e->d_name);
		struct stat st;
		if (lstat(h, &st)) {
			continue;
		}
		if (S_ISDIR(st.st_mode)) {
			walk(h, g);
		} else if (S_ISREG(st.st_mode)) {
			struct e4_inode in;
			int err = e4_lookup(&fs, g, &in);
			nfiles++;
			if (err || in.size != (uint64_t)st.st_size) {
				printf("FAIL %s: lookup err %d size %llu want %lld\n", g, err, err ? 0 : in.size, (long long)st.st_size);
				nbad++;
				continue;
			}
			uint8_t *a = malloc(st.st_size + 1), *b = malloc(st.st_size + 1);
			FILE *f = fopen(h, "rb");
			size_t n = fread(a, 1, st.st_size, f);
			fclose(f);
			uint64_t got = 0;
			/* odd-sized reads from odd offsets too */
			int rc = e4_read(&fs, &in, 0, b, st.st_size, &got);
			if (rc || got != (uint64_t)st.st_size || n != (size_t)st.st_size || memcmp(a, b, st.st_size)) {
				printf("FAIL %s: read rc %d got %llu\n", g, rc, got);
				nbad++;
			} else if (st.st_size > 5000) {
				uint8_t part[3001];
				rc = e4_read(&fs, &in, 4999, part, 3001, &got);
				if (rc || got != 3001 || memcmp(part, a + 4999, 3001)) {
					printf("FAIL %s: partial read\n", g);
					nbad++;
				}
			}
			free(a);
			free(b);
		}
	}
	if (d) {
		closedir(d);
	}
}

int
main(int argc, char **argv)
{
	if (argc < 3) {
		return 2;
	}
	img = fopen(argv[1], "rb");
	if (!img) {
		perror(argv[1]);
		return 2;
	}
	struct e4_dev base = { rd, NULL }, dev = base;
	struct lp_view v = { { &base, NULL, NULL, NULL }, { { 0, 0, 0 } }, 0 };
	if (argc >= 5) {
		fclose(img);
		img = fopen(argv[3], "rb");
		unsigned n = 0;
		int err = lp_find(&base, argv[4], v.ext, 8, &n, NULL);
		char mg[48];
		fs_magic(&base, mg, sizeof(mg));
		printf("super magic: %s\n", mg);
		printf("lp_find %s: err %d, %u extents", argv[4], err, n);
		for (unsigned i = 0; i < n; i++) {
			printf(" [%llu+%llu]", v.ext[i].first_sector, v.ext[i].sectors);
		}
		printf("\n");
		if (err) {
			return 1;
		}
		v.n = n;
		dev.read = lp_read;
		dev.ctx = &v;
	}
	int err = e4_mount(&fs, &dev);
	printf("mount: err %d bs %u groups %u inode %u desc %u incompat %#x label '%s'\n", err, fs.bs, fs.ngroups, fs.inode_size,
	    fs.desc_size, fs.incompat, fs.label);
	if (err) {
		return 1;
	}
	struct e4_inode root;
	e4_lookup(&fs, "/", &root);
	e4_readdir(&fs, &root, list_cb, NULL);
	walk(argv[2], "");
	{
		struct e4_inode in;
		uint8_t b[16];
		uint64_t got = 0;
		int a = e4_lookup(&fs, "/link_abs/c/leaf", &in);
		int r = a ? a : e4_read(&fs, &in, 0, b, sizeof(b), &got);
		int c = e4_lookup(&fs, "/link_rel/config/long_file_name_for_hashing_7.json", &in);
		printf("symlinks: abs %d (%llu bytes, %.5s) rel %d\n", r, got, a ? "" : (char *)b, c);
		nbad += (r || got != 6 || memcmp(b, "hello\n", 6) || c);
	}
	printf("%u files compared, %u bad\n", nfiles, nbad);
	e4_unmount(&fs);
	return nbad != 0;
}
