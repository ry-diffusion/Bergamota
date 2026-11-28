/*
 * BERGAMOTA read-only ext2/3/4 reader, plus an Android "super" (liblp) partition lookup. Reads only, never writes: it
 * is for the SLPI's data files (persist/sensors, dsp, vendor/etc/sensors), docs/SENSORS_SLPI.md. The same source is the
 * kernel file bsd/dev/bergamota_ext4.c and the host test tools/ext4test (-DE4_HOST_TEST).
 *
 * Supported: 1-64 KiB blocks, extents and classic block maps, 64bit, flex_bg, filetype, sparse files, htree
 * directories (scanned linearly), fast and slow symlinks. Refused with EOPNOTSUPP: meta_bg, inline_data inodes,
 * encrypted inodes. Journals and checksums are ignored (a dirty journal is reported, not replayed).
 */
#if defined(E4_HOST_TEST)
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define E4_ALLOC(n)     calloc(1, (n))
#define E4_FREE(p, n)   free(p)
#define E4_MIN(a, b)    ((a) < (b) ? (a) : (b))
#elif defined(ARM64_BOARD_CONFIG_BERGAMOTA)
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/errno.h>
#include <kern/kalloc.h>
#define E4_ALLOC(n)     kalloc_data((n), Z_WAITOK | Z_ZERO)
#define E4_FREE(p, n)   kfree_data((p), (n))
#define E4_MIN(a, b)    ((a) < (b) ? (a) : (b))
#endif

#if defined(E4_HOST_TEST) || defined(ARM64_BOARD_CONFIG_BERGAMOTA)
#include "bergamota_ext4.h"

static uint32_t
e4_le16(const uint8_t *p)
{
	return (uint32_t)p[0] | ((uint32_t)p[1] << 8);
}

static uint32_t
e4_le32(const uint8_t *p)
{
	return e4_le16(p) | (e4_le16(p + 2) << 16);
}

static uint64_t
e4_le64(const uint8_t *p)
{
	return e4_le32(p) | ((uint64_t)e4_le32(p + 4) << 32);
}

#define E4_MAGIC        0xef53u
#define FL_EXTENTS      0x00080000u
#define FL_INLINE       0x10000000u
#define FL_ENCRYPT      0x00000800u
#define INC_META_BG     0x0010u
#define INC_64BIT       0x0080u
#define INC_RECOVER     0x0004u

int
e4_mount(struct e4_fs *fs, const struct e4_dev *dev)
{
	uint8_t sb[1024];
	memset(fs, 0, sizeof(*fs));
	fs->dev = dev;
	int err = dev->read(dev->ctx, 1024, sb, sizeof(sb));
	if (err) {
		return err;
	}
	if (e4_le16(sb + 56) != E4_MAGIC) {
		return ENXIO;
	}
	uint32_t lbs = e4_le32(sb + 24);
	if (lbs > 6) {
		return EINVAL;
	}
	fs->bs = 1024u << lbs;
	fs->bpg = e4_le32(sb + 32);
	fs->ipg = e4_le32(sb + 40);
	fs->first_data = e4_le32(sb + 20);
	fs->inode_size = e4_le32(sb + 76) >= 1 ? e4_le16(sb + 88) : 128;
	fs->compat = e4_le32(sb + 92);
	fs->incompat = e4_le32(sb + 96);
	fs->ro_compat = e4_le32(sb + 100);
	fs->blocks = e4_le32(sb + 4);
	if (fs->incompat & INC_64BIT) {
		fs->blocks |= (uint64_t)e4_le32(sb + 336) << 32;
		fs->desc_size = e4_le16(sb + 254);
	} else {
		fs->desc_size = 32;
	}
	memcpy(fs->label, sb + 120, 16);
	fs->label[16] = 0;
	if (!fs->bpg || !fs->ipg || fs->inode_size < 128 || fs->inode_size > 1024 || fs->desc_size < 32 ||
	    fs->desc_size > 1024) {
		return EINVAL;
	}
	if (fs->incompat & INC_META_BG) {
		return EOPNOTSUPP;
	}
	fs->ngroups = (uint32_t)((fs->blocks - fs->first_data + fs->bpg - 1) / fs->bpg);
	fs->dirty_journal = (fs->incompat & INC_RECOVER) != 0;
	fs->blk = E4_ALLOC(fs->bs);
	fs->blk2 = E4_ALLOC(fs->bs);
	if (!fs->blk || !fs->blk2) {
		e4_unmount(fs);
		return ENOMEM;
	}
	return 0;
}

void
e4_unmount(struct e4_fs *fs)
{
	if (fs->blk) {
		E4_FREE(fs->blk, fs->bs);
	}
	if (fs->blk2) {
		E4_FREE(fs->blk2, fs->bs);
	}
	fs->blk = fs->blk2 = NULL;
}

static int
e4_rd(struct e4_fs *fs, uint64_t blk, uint32_t off, void *dst, uint32_t len)
{
	return fs->dev->read(fs->dev->ctx, blk * fs->bs + off, dst, len);
}

int
e4_get_inode(struct e4_fs *fs, uint32_t ino, struct e4_inode *out)
{
	uint8_t d[64], raw[256];
	if (ino == 0 || ino > (uint64_t)fs->ipg * fs->ngroups) {
		return EINVAL;
	}
	uint32_t grp = (ino - 1) / fs->ipg, idx = (ino - 1) % fs->ipg;
	uint64_t gdt = (uint64_t)fs->first_data + 1;
	uint32_t per = fs->bs / fs->desc_size;
	int err = e4_rd(fs, gdt + grp / per, (grp % per) * fs->desc_size, d, E4_MIN(fs->desc_size, 64u));
	if (err) {
		return err;
	}
	uint64_t itable = e4_le32(d + 8);
	if ((fs->incompat & INC_64BIT) && fs->desc_size >= 64) {
		itable |= (uint64_t)e4_le32(d + 0x28) << 32;
	}
	if (!itable) {
		return EIO;
	}
	err = fs->dev->read(fs->dev->ctx, itable * fs->bs + (uint64_t)idx * fs->inode_size, raw, E4_MIN(fs->inode_size, 256u));
	if (err) {
		return err;
	}
	memset(out, 0, sizeof(*out));
	out->ino = ino;
	out->mode = (uint16_t)e4_le16(raw);
	out->flags = e4_le32(raw + 0x20);
	out->size = e4_le32(raw + 4);
	if ((out->mode & 0xf000) == 0x8000) {
		out->size |= (uint64_t)e4_le32(raw + 0x6c) << 32;
	}
	memcpy(out->iblock, raw + 0x28, 60);
	if (out->flags & (FL_INLINE | FL_ENCRYPT)) {
		return EOPNOTSUPP;
	}
	return 0;
}

/*
 * Map logical block lblk of an inode: *pblk = physical block (0 = hole), *run = how many following blocks share the
 * mapping (contiguous, or all holes). Extent trees of any depth, or the classic 12 + 3 indirect map.
 */
static int
e4_map(struct e4_fs *fs, const struct e4_inode *in, uint64_t lblk, uint64_t *pblk, uint64_t *run)
{
	*pblk = 0;
	*run = 1;
	if (in->flags & FL_EXTENTS) {
		const uint8_t *node = in->iblock;
		uint32_t nodelen = 60;
		for (unsigned depth = 0; depth < 8; depth++) {
			if (nodelen < 12 || e4_le16(node) != 0xf30a) {
				return EIO;
			}
			uint32_t n = e4_le16(node + 2), dep = e4_le16(node + 6);
			if (12 + n * 12 > nodelen) {
				return EIO;
			}
			if (dep == 0) {
				uint64_t next = UINT64_MAX;
				for (uint32_t k = 0; k < n; k++) {
					const uint8_t *e = node + 12 + 12 * k;
					uint64_t first = e4_le32(e), len = e4_le16(e + 4);
					if (len > 32768) {
						len -= 32768;
					}
					if (lblk >= first && lblk < first + len) {
						*pblk = (e4_le32(e + 8) | ((uint64_t)e4_le16(e + 6) << 32)) + (lblk - first);
						*run = first + len - lblk;
						return 0;
					}
					if (first > lblk && first < next) {
						next = first;
					}
				}
				*run = next == UINT64_MAX ? 0x7fffffffu : next - lblk;
				return 0;
			}
			const uint8_t *pick = NULL;
			for (uint32_t k = 0; k < n; k++) {
				const uint8_t *e = node + 12 + 12 * k;
				if (e4_le32(e) <= lblk) {
					pick = e;
				}
			}
			if (!pick) {
				*run = 0x7fffffffu;
				return 0;
			}
			uint64_t child = e4_le32(pick + 4) | ((uint64_t)e4_le16(pick + 8) << 32);
			int err = e4_rd(fs, child, 0, fs->blk2, fs->bs);
			if (err) {
				return err;
			}
			node = fs->blk2;
			nodelen = fs->bs;
		}
		return EIO;
	}
	uint32_t apb = fs->bs / 4;
	if (lblk < 12) {
		*pblk = e4_le32(in->iblock + 4 * lblk);
		return 0;
	}
	lblk -= 12;
	uint64_t b;
	unsigned levels;
	if (lblk < apb) {
		b = e4_le32(in->iblock + 48);
		levels = 1;
	} else if ((lblk -= apb) < (uint64_t)apb * apb) {
		b = e4_le32(in->iblock + 52);
		levels = 2;
	} else {
		lblk -= (uint64_t)apb * apb;
		if (lblk >= (uint64_t)apb * apb * apb) {
			return EFBIG;
		}
		b = e4_le32(in->iblock + 56);
		levels = 3;
	}
	for (unsigned l = levels; l > 0 && b; l--) {
		uint64_t span = 1;
		for (unsigned k = 1; k < l; k++) {
			span *= apb;
		}
		uint32_t slot = (uint32_t)(lblk / span);
		lblk %= span;
		uint8_t w[4];
		int err = e4_rd(fs, b, slot * 4, w, 4);
		if (err) {
			return err;
		}
		b = e4_le32(w);
	}
	*pblk = b;
	return 0;
}

int
e4_read(struct e4_fs *fs, const struct e4_inode *in, uint64_t off, void *dst, uint64_t len, uint64_t *got)
{
	uint8_t *d = dst;
	uint64_t done = 0;
	*got = 0;
	if (off >= in->size) {
		return 0;
	}
	len = E4_MIN(len, in->size - off);
	while (done < len) {
		uint64_t pos = off + done, lblk = pos / fs->bs, inb = pos % fs->bs, pblk, run;
		int err = e4_map(fs, in, lblk, &pblk, &run);
		if (err) {
			return err;
		}
		uint64_t n = E4_MIN(len - done, run * fs->bs - inb);
		if (pblk == 0) {
			memset(d + done, 0, n);
		} else {
			n = E4_MIN(n, 1u << 20);
			err = fs->dev->read(fs->dev->ctx, pblk * fs->bs + inb, d + done, n);
			if (err) {
				return err;
			}
		}
		done += n;
		*got = done;
	}
	return 0;
}

/* cb(name, namelen, ino, type, ctx) -> nonzero stops; type is the dirent file_type (1 file, 2 dir, 7 symlink) or 0 */
int
e4_readdir(struct e4_fs *fs, const struct e4_inode *dir, int (*cb)(const char *, unsigned, uint32_t, unsigned, void *),
    void *ctx)
{
	if ((dir->mode & 0xf000) != 0x4000) {
		return ENOTDIR;
	}
	for (uint64_t pos = 0; pos < dir->size; pos += fs->bs) {
		uint64_t got;
		int err = e4_read(fs, dir, pos, fs->blk, fs->bs, &got);
		if (err) {
			return err;
		}
		if (got < fs->bs) {
			break;
		}
		for (uint32_t o = 0; o + 8 <= fs->bs;) {
			uint32_t ino = e4_le32(fs->blk + o), rl = e4_le16(fs->blk + o + 4), nl = fs->blk[o + 6];
			if (rl < 8 || (rl & 3) || o + rl > fs->bs || 8 + nl > rl) {
				break;
			}
			if (ino && nl) {
				if (cb((const char *)fs->blk + o + 8, nl, ino, fs->blk[o + 7], ctx)) {
					return 0;
				}
			}
			o += rl;
		}
	}
	return 0;
}

struct e4_find {
	const char *name;
	unsigned len;
	uint32_t ino;
};

static int
e4_find_cb(const char *n, unsigned l, uint32_t ino, unsigned type, void *ctx)
{
	struct e4_find *f = ctx;
	(void)type;
	if (l == f->len && memcmp(n, f->name, l) == 0) {
		f->ino = ino;
		return 1;
	}
	return 0;
}

static int
e4_symlink_target(struct e4_fs *fs, const struct e4_inode *in, char *buf, unsigned max)
{
	if (in->size == 0 || in->size >= max) {
		return ENAMETOOLONG;
	}
	if (in->size < 60 && !((in->flags & FL_EXTENTS) && e4_le16(in->iblock + 2))) {
		memcpy(buf, in->iblock, (size_t)in->size);
		buf[in->size] = 0;
		return 0;
	}
	uint64_t got;
	int err = e4_read(fs, in, 0, buf, in->size, &got);
	buf[in->size] = 0;
	return err;
}

/* absolute path -> inode, following symlinks (8 deep) */
int
e4_lookup(struct e4_fs *fs, const char *path, struct e4_inode *out)
{
	char cur[256];
	unsigned links = 0;
	if (strlen(path) >= sizeof(cur)) {
		return ENAMETOOLONG;
	}
	strlcpy(cur, path, sizeof(cur));
	struct e4_inode in;
	int err = e4_get_inode(fs, 2, &in);
	if (err) {
		return err;
	}
	uint32_t parents[16];
	unsigned depth = 0;
	char *p = cur;
	while (*p) {
		while (*p == '/') {
			p++;
		}
		if (!*p) {
			break;
		}
		char *s = p;
		while (*p && *p != '/') {
			p++;
		}
		unsigned l = (unsigned)(p - s);
		if (l == 1 && s[0] == '.') {
			continue;
		}
		if (l == 2 && s[0] == '.' && s[1] == '.') {
			if (depth) {
				err = e4_get_inode(fs, parents[--depth], &in);
				if (err) {
					return err;
				}
			}
			continue;
		}
		struct e4_find f = { s, l, 0 };
		err = e4_readdir(fs, &in, e4_find_cb, &f);
		if (err) {
			return err;
		}
		if (!f.ino) {
			return ENOENT;
		}
		uint32_t parent = in.ino;
		err = e4_get_inode(fs, f.ino, &in);
		if (err) {
			return err;
		}
		if ((in.mode & 0xf000) == 0xa000) {
			char tgt[256], rest[256];
			if (++links > 8) {
				return ELOOP;
			}
			err = e4_symlink_target(fs, &in, tgt, sizeof(tgt));
			if (err) {
				return err;
			}
			strlcpy(rest, p, sizeof(rest));
			if (strlen(tgt) + strlen(rest) + 2 > sizeof(cur)) {
				return ENAMETOOLONG;
			}
			if (tgt[0] == '/') {
				depth = 0;
				err = e4_get_inode(fs, 2, &in);
			} else {
				err = e4_get_inode(fs, parent, &in);
			}
			if (err) {
				return err;
			}
			snprintf(cur, sizeof(cur), "%s/%s", tgt, rest);
			p = cur;
			continue;
		}
		if (depth < 16) {
			parents[depth++] = parent;
		}
	}
	*out = in;
	return 0;
}

/* ---- Android dynamic partitions (liblp): where a logical partition sits inside "super" */
int
lp_find(const struct e4_dev *super, const char *name, struct lp_extent *ext, unsigned max, unsigned *n)
{
	uint8_t g[64];
	*n = 0;
	int err = super->read(super->ctx, 4096, g, sizeof(g));
	if (err) {
		return err;
	}
	if (e4_le32(g) != 0x616c4467u) {
		return ENXIO;
	}
	uint32_t max_meta = e4_le32(g + 40), slots = e4_le32(g + 44);
	if (max_meta < 256 || max_meta > 0x100000u || slots == 0 || slots > 4) {
		return EINVAL;
	}
	uint8_t *m = E4_ALLOC(max_meta);
	if (!m) {
		return ENOMEM;
	}
	int rc = ENOENT;
	for (uint32_t slot = 0; slot < slots && rc == ENOENT; slot++) {
		err = super->read(super->ctx, 12288 + (uint64_t)slot * max_meta, m, max_meta);
		if (err) {
			rc = err;
			break;
		}
		if (e4_le32(m) != 0x414c5030u) {
			continue;
		}
		uint32_t hs = e4_le32(m + 8);
		uint32_t po = e4_le32(m + 80), pn = e4_le32(m + 84), ps = e4_le32(m + 88);
		uint32_t eo = e4_le32(m + 92), en = e4_le32(m + 96), es = e4_le32(m + 100);
		if (ps < 52 || es < 24 || hs > max_meta || (uint64_t)hs + po + (uint64_t)pn * ps > max_meta ||
		    (uint64_t)hs + eo + (uint64_t)en * es > max_meta) {
			continue;
		}
		for (uint32_t i = 0; i < pn; i++) {
			const uint8_t *pe = m + hs + po + (uint64_t)i * ps;
			char nm[37];
			memcpy(nm, pe, 36);
			nm[36] = 0;
			if (strcmp(nm, name)) {
				continue;
			}
			uint32_t first = e4_le32(pe + 40), cnt = e4_le32(pe + 44);
			if (cnt > max || first + cnt > en) {
				rc = EFBIG;
				break;
			}
			rc = 0;
			for (uint32_t k = 0; k < cnt; k++) {
				const uint8_t *ee = m + hs + eo + (uint64_t)(first + k) * es;
				if (e4_le32(ee + 8) != 0 || e4_le32(ee + 20) != 0) {
					rc = EOPNOTSUPP;      /* zero targets / other block devices */
					break;
				}
				ext[k].first_sector = e4_le64(ee + 12);
				ext[k].sectors = e4_le64(ee);
			}
			*n = rc ? 0 : cnt;
			break;
		}
	}
	E4_FREE(m, max_meta);
	return rc;
}

/* a device made of lp extents over a base device (512-byte sectors) */
int
lp_read(void *ctx, uint64_t off, void *dst, uint64_t len)
{
	struct lp_view *v = ctx;
	uint8_t *d = dst;
	while (len) {
		uint64_t base = 0, left = 0;
		unsigned i;
		for (i = 0; i < v->n; i++) {
			uint64_t bytes = v->ext[i].sectors * 512;
			if (off < base + bytes) {
				left = base + bytes - off;
				break;
			}
			base += bytes;
		}
		if (i == v->n) {
			return EINVAL;
		}
		uint64_t take = E4_MIN(len, left);
		int err = v->base->read(v->base->ctx, v->ext[i].first_sector * 512 + (off - base), d, take);
		if (err) {
			return err;
		}
		d += take;
		off += take;
		len -= take;
	}
	return 0;
}

#endif
