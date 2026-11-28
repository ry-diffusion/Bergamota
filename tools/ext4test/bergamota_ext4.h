/* bergamota_ext4.h: read-only ext4 + liblp lookup, see bergamota_ext4.c */
#ifndef BERGAMOTA_EXT4_H
#define BERGAMOTA_EXT4_H

struct e4_dev {
	int (*read)(void *ctx, uint64_t off, void *dst, uint64_t len);    /* byte offsets, 0 or errno */
	void *ctx;
};

struct e4_fs {
	const struct e4_dev *dev;
	uint32_t bs, bpg, ipg, inode_size, desc_size, first_data, ngroups, compat, incompat, ro_compat;
	uint64_t blocks;
	bool dirty_journal;
	char label[17];
	uint8_t *blk, *blk2;                    /* scratch blocks (directory scan / index nodes) */
};

struct e4_inode {
	uint32_t ino, flags;
	uint16_t mode;
	uint64_t size;
	uint8_t iblock[60];
};

struct lp_extent {
	uint64_t first_sector, sectors;
	uint32_t dev;                           /* index into the metadata's block devices (0 = the device it was read from) */
};

struct lp_info {
	uint32_t nbdev;
	char bdev[4][36];                       /* block device (GPT partition) names, as in a retrofit layout */
};

struct lp_view {
	const struct e4_dev *devs[4];           /* devs[extent.dev] */
	struct lp_extent ext[8];
	unsigned n;
};

int e4_mount(struct e4_fs *fs, const struct e4_dev *dev);
void e4_unmount(struct e4_fs *fs);
int e4_get_inode(struct e4_fs *fs, uint32_t ino, struct e4_inode *out);
int e4_read(struct e4_fs *fs, const struct e4_inode *in, uint64_t off, void *dst, uint64_t len, uint64_t *got);
int e4_readdir(struct e4_fs *fs, const struct e4_inode *dir, int (*cb)(const char *, unsigned, uint32_t, unsigned, void *),
    void *ctx);
int e4_lookup(struct e4_fs *fs, const char *path, struct e4_inode *out);
int lp_find(const struct e4_dev *super, const char *name, struct lp_extent *ext, unsigned max, unsigned *n,
    struct lp_info *info);
/* cb(name, extents, sectors, attributes, first extent's device, ctx) for every logical partition */
int lp_list(const struct e4_dev *super, void (*cb)(const char *, uint32_t, uint64_t, uint32_t, uint32_t, void *), void *ctx);
/* what is at the start of a device: "ext4", "erofs", "f2fs", "squashfs", "liblp" or "unknown ..." */
int fs_magic(const struct e4_dev *dev, char *out, unsigned max);
int lp_read(void *ctx, uint64_t off, void *dst, uint64_t len);

#endif
