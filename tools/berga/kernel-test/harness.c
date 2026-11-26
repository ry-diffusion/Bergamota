/*
 * Host harness for the kernel's berga code (xnu patch 1037): the block is cut out of the patch itself
 * (run.sh), compiled against these xnu stubs with ASan/UBSan, and driven like the USB controller would:
 * HELLO, daemon routing through /dev/berga, transfers holding 2 frames or half a frame, PAD, the log stream,
 * reboot modes, garbage, bus reset. The frames the kernel produced go to argv[1] for the Rust decoder test.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>
#include <assert.h>
#include <sys/param.h>
#include <errno.h>
#include <time.h>
typedef struct { int x; } lck_mtx_t;
typedef int wait_result_t; typedef void *thread_t; typedef int kern_return_t;
struct proc; struct uio { uint8_t *p; long resid; int w; };
#define KERN_SUCCESS 0
#define RB_AUTOBOOT 0
#define RB_HALT 8
#define PZERO 22
#define PCATCH 0x100
#define Z_WAITOK 0
#define Z_ZERO 1
#define UID_ROOT 0
#define GID_WHEEL 0
#define DEVFS_CHAR 0
#define LCK_GRP_ATTR_NULL 0
#define LCK_ATTR_NULL 0
#define SYSCTL_HANDLER_ARGS (void *oidp, void *arg1, int arg2, void *req)
#define SYSCTL_OUT(r, b, n) ((void)(b), (int)(n))
#define SYSCTL_PROC(...)
#define DEPCMD_SETEPCONFIG 1
#define DEPCMD_SETTRANSFRESOURCE 2
#define TRBCTL_NORMAL (1u << 4)
#define DALEPENA 0x720
static char version[] = "Darwin Kernel Version 24.6.0: \"harness\"; root:xnu-11215.81.4~1/RELEASE_ARM64_BERGAMOTA";
static struct { uintptr_t dma; bool configured; bool xfer_active[8]; uint64_t rx_frames, tx_frames; uint32_t resets; } ud;
#define UD_TRB_BG_RX 0x1060
#define UD_TRB_BG_TX 0x1070
#define UD_BG_RX 0x4000
#define UD_BG_TX 0x14000
#define UD_BG_BUF 0x10000
#define EP_BG_RX 6
#define EP_BG_TX 7
static uint32_t last_tx_len; static int rebooted = -1; static char sysctl_mode[32];
static void ud_sync(uint32_t o, uint32_t l) { (void)o; (void)l; }
static void ud_start_trb(unsigned ep, uint32_t trb, uint32_t buf, uint32_t len, uint32_t ctl) { (void)trb; (void)buf; (void)ctl; ud.xfer_active[ep] = true; if (ep == EP_BG_TX) last_tx_len = len; }
static int ud_epcmd(unsigned ep, uint32_t c, uint32_t a, uint32_t b, uint32_t d) { (void)ep; (void)c; (void)a; (void)b; (void)d; return 0; }
static void dwc_wr(uint32_t o, uint32_t v) { (void)o; (void)v; }
static uint32_t dwc_rd(uint32_t o) { (void)o; return 0; }
static void ud_evlog(uint32_t v) { (void)v; }
static void ud_sleep_ms(unsigned ms) { (void)ms; }
static void lck_mtx_lock(lck_mtx_t *l) { (void)l; }
static void lck_mtx_unlock(lck_mtx_t *l) { (void)l; }
static lck_mtx_t the_lock; static void *lck_grp_alloc_init(const char *n, int a) { (void)n; (void)a; return &the_lock; }
static lck_mtx_t *lck_mtx_alloc_init(void *g, int a) { (void)g; (void)a; return &the_lock; }
static void *kalloc_data(size_t n, int f) { return f & Z_ZERO ? calloc(1, n) : malloc(n); }
static void kfree_data(void *p, size_t n) { (void)n; free(p); }
static void wakeup(void *c) { (void)c; }
static int msleep(void *c, lck_mtx_t *m, int p, const char *w, struct timespec *t) { (void)c; (void)m; (void)p; (void)w; (void)t; return EWOULDBLOCK; }
static long uio_resid(struct uio *u) { return u->resid; }
static int uiomove(const char *cp, int n, struct uio *u) { if (u->w) memcpy((char *)cp, u->p, n); else memcpy(u->p, cp, n); u->resid -= n; return 0; }
static uint64_t mach_absolute_time(void) { return 123456789; }
static void absolutetime_to_nanoseconds(uint64_t t, uint64_t *ns) { *ns = t * 1000; }
static uint64_t logpos = 100000; static const char *logtext = "BERGAMOTA: hello from the log ring\n";
uint32_t bergamota_log_copy(uint64_t *pos, char *out, uint32_t max) { if (*pos >= logpos) return 0; uint32_t n = (uint32_t)MIN((uint64_t)max, logpos - *pos); for (uint32_t i = 0; i < n; i++) out[i] = logtext[(*pos + i) % strlen(logtext)]; *pos += n; return n; }
uint64_t bergamota_log_position(void) { return logpos; }
int kernel_sysctlbyname(const char *name, void *o, size_t *ol, void *n, size_t nl) { (void)o; (void)ol; assert(!strcmp(name, "kern.bergamota_reboot_mode")); memcpy(sysctl_mode, n, nl); return 0; }
static kern_return_t kernel_thread_start(void (*f)(void *, wait_result_t), void *a, thread_t *t) { (void)t; f(a, 0); return 0; }
static void thread_deallocate(thread_t t) { (void)t; }
int reboot_kernel(int howto, char *m) { (void)m; rebooted = howto; return 0; }
static int PE_parse_boot_argn(const char *n, void *v, int s) { (void)n; (void)v; (void)s; return 0; }
static int eno_ioctl, eno_stop, eno_reset, eno_select, eno_mmap, eno_strat, eno_getc, eno_putc;
struct cdevsw { void *d_open, *d_close, *d_read, *d_write; void *d_ioctl, *d_stop, *d_reset, *d_ttys, *d_select, *d_mmap, *d_strategy, *d_reserved_1, *d_reserved_2; int d_type; };
#define eno_ioctl ((void *)&eno_ioctl)
#define eno_stop ((void *)&eno_stop)
#define eno_reset ((void *)&eno_reset)
#define eno_select ((void *)&eno_select)
#define eno_mmap ((void *)&eno_mmap)
#define eno_strat ((void *)&eno_strat)
#define eno_getc ((void *)&eno_getc)
#define eno_putc ((void *)&eno_putc)
static int cdevsw_add(int m, const struct cdevsw *c) { (void)m; (void)c; return 42; }
static void devfs_make_node(int d, int t, int u, int g, int p, const char *n) { (void)d; (void)t; (void)u; (void)g; (void)p; (void)n; }
#define printf(...) fprintf(stderr, __VA_ARGS__)
#undef __unused
#define __unused __attribute__((unused))
#include BERGA_BLOCK

/* host side: build a request like berga-core does */
static uint32_t mk(uint8_t *f, uint16_t op, uint32_t tag, const char *head, const char *body)
{
	uint32_t hl = head ? strlen(head) : 0, bl = body ? strlen(body) : 0, n = BG_HDR + hl + bl;
	bg_put32(f, BG_MAGIC); f[4] = 1; f[5] = (n % 512 == 0); f[6] = op; f[7] = op >> 8;
	bg_put32(f + 8, tag); bg_put32(f + 12, hl); bg_put32(f + 16, bl);
	memcpy(f + 20, head, hl); memcpy(f + 20 + hl, body, bl);
	if (f[5]) f[n++] = 0;
	return n;
}
static void host_send(const uint8_t *f, uint32_t n) /* one bulk OUT transfer completes */
{
	memcpy((void *)(ud.dma + UD_BG_RX), f, n);
	*(uint32_t *)(ud.dma + UD_TRB_BG_RX + 8) = UD_BG_BUF - n;
	bg_rx_done();
}
/* next frame the phone puts on bulk IN */
static uint8_t *host_recv(uint32_t *n)
{
	bg_poll();
	if (!bg.tx_busy) return NULL;
	*n = last_tx_len;
	bg.tx_busy = false;     /* XferComplete */
	return (uint8_t *)(ud.dma + UD_BG_TX);
}
int main(int argc, char **argv)
{
	FILE *out = argc > 1 ? fopen(argv[1], "wb") : NULL;
	uint8_t f[BG_MAX], g[BG_MAX]; uint32_t n;
	ud.dma = (uintptr_t)calloc(1, 0x24000); ud.configured = true;
	bg_init();
	assert(bg.asm_buf);
	/* HELLO */
	host_send(f, mk(f, BG_OP_HELLO, 7, NULL, NULL));
	uint8_t *r = host_recv(&n); assert(r && r[4] == BG_RESPONSE && bg_le32(r + 8) == 7);
	if (out) fwrite(r, 1, n, out);
	fprintf(stderr, "hello head: %.*s\n", (int)bg_le32(r + 12), r + 20);
	/* daemon op without daemon -> error */
	host_send(f, mk(f, 0x0100, 8, NULL, NULL));
	r = host_recv(&n); assert(r && r[4] == BG_ERROR && strstr((char *)r + 20, "no daemon"));
	if (out) fwrite(r, 1, n, out);
	/* daemon attached: request goes to /dev/berga, response comes back */
	assert(bg_dev_open(0, 0, 0, NULL) == 0 && bg_dev_open(0, 0, 0, NULL) == EBUSY);
	uint32_t rn = mk(f, 0x0102, 9, "{\"cmd\":\"uptime\"}", NULL);
	/* two frames in ONE transfer + a frame split over two transfers */
	uint32_t rn2 = mk(g, 0x0100, 10, NULL, NULL);
	uint8_t both[200]; memcpy(both, f, rn); memcpy(both + rn, g, 5); host_send(both, rn + 5); host_send(g + 5, rn2 - 5);
	uint8_t rb[BG_MAX]; struct uio u = { rb, BG_MAX };
	assert(bg_dev_read(0, &u, 0) == 0 && BG_MAX - u.resid == rn && bg_le32(rb + 8) == 9);
	u = (struct uio){ rb, BG_MAX };
	assert(bg_dev_read(0, &u, 0) == 0 && bg_le32(rb + 8) == 10);
	u = (struct uio){ rb, 100 }; assert(bg_dev_read(0, &u, 0) == EINVAL);
	/* daemon writes a response: must be a whole frame of kind 2..4 */
	uint32_t wn = mk(rb, 0x0102, 9, "{\"code\":0}", "up 2 mins\n"); rb[4] = BG_RESPONSE;
	u = (struct uio){ rb, wn, 1 }; assert(bg_dev_write(0, &u, 0) == 0);
	r = host_recv(&n); assert(r && n == wn && bg_le32(r + 8) == 9);
	if (out) fwrite(r, 1, n, out);
	rb[4] = BG_REQUEST; u = (struct uio){ rb, wn, 1 }; assert(bg_dev_write(0, &u, 0) == EINVAL);
	rb[4] = BG_RESPONSE; u = (struct uio){ rb, wn - 1, 1 }; assert(bg_dev_write(0, &u, 0) == EINVAL);
	/* a 512-multiple frame from the daemon carries its PAD byte */
	char big[600]; memset(big, 'x', 512 - 20); big[492] = 0;
	wn = mk(rb, 0x0104, 11, NULL, big); rb[4] = BG_RESPONSE; assert(wn == 513);
	u = (struct uio){ rb, wn, 1 }; assert(bg_dev_write(0, &u, 0) == 0);
	r = host_recv(&n); assert(n == 513 && (r[5] & BG_FLAG_PAD));
	if (out) fwrite(r, 1, n, out);
	/* log stream: response then events */
	host_send(f, mk(f, BG_OP_LOG_START, 12, NULL, "1000"));
	r = host_recv(&n); assert(r && r[4] == BG_RESPONSE && bg_le32(r + 8) == 12);
	if (out) fwrite(r, 1, n, out);
	r = host_recv(&n); assert(r && r[4] == BG_EVENT && bg_le32(r + 8) == 12 && bg_le32(r + 16) == 1000);
	if (out) fwrite(r, 1, n, out);
	host_send(f, mk(f, BG_OP_LOG_STOP, 13, NULL, NULL));
	r = host_recv(&n); assert(r && bg_le32(r + 8) == 13);
	/* reboot */
	host_send(f, mk(f, BG_OP_REBOOT, 14, NULL, "fastboot"));
	r = host_recv(&n); assert(r && r[4] == BG_RESPONSE && rebooted == RB_AUTOBOOT && !strcmp(sysctl_mode, "fastboot"));
	if (out) fwrite(r, 1, n, out);
	host_send(f, mk(f, BG_OP_REBOOT, 15, NULL, "poweroff"));
	r = host_recv(&n); assert(rebooted == RB_HALT);
	rebooted = -1; host_send(f, mk(f, BG_OP_REBOOT, 16, NULL, "edl"));
	r = host_recv(&n); assert(r[4] == BG_ERROR && rebooted == -1);
	/* garbage resynchronises */
	host_send((const uint8_t *)"GET / HTTP/1.1\r\n\r\n12345", 23);
	assert(bg.asm_len == 0);
	host_send(f, mk(f, BG_OP_HELLO, 17, NULL, NULL));
	r = host_recv(&n); assert(r && bg_le32(r + 8) == 17);
	/* reset drops queued output */
	bg_dev_close(0, 0, 0, NULL);
	host_send(f, mk(f, 0x0100, 18, NULL, NULL)); bg_reset(); assert(!host_recv(&n));
	fprintf(stderr, "harness OK: in=%u out=%u to_daemon=%u drops=%u\n", bg.frames_in, bg.frames_out, bg.to_daemon, bg.drops);
	if (out) fclose(out);
	return 0;
}
