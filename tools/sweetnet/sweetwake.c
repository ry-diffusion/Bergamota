/*
 * sweetwake: wake, unlock and drive the iOS 26 home screen over SSH, so GPU tests need no hands on the phone.
 *   sweetwake wake                         undim the display (SpringBoardServices SBSUndimScreen; Home press if missing)
 *   sweetwake unlock                       wake, then swipe up from the bottom edge (no passcode on this phone)
 *   sweetwake pages [N]                    swipe to the next home page and back, N times (default 1)
 *   sweetwake awake SECONDS                keep the display on: undim every 10 s for SECONDS, in the background
 *   sweetwake tap X Y | swipe X1 Y1 X2 Y2 [MS] | home | lock
 *   sweetwake hold X1 Y1 X2 Y2 HOLD_MS [MS]   finger down at X1 Y1 for HOLD_MS, then moved to X2 Y2 over MS, held there
 *                                          a moment, lifted (long-press drags: an icon onto another makes a folder)
 *   sweetwake drag X1 X2 Y SECONDS [MS [STEP_MS]]  finger down for SECONDS, sweeping X1<->X2 (MS each way, a move every
 *                                          STEP_MS, default 8); run it under fgjob
 * Points in the 414x896 space of the iPhone12,1 userland. Events go into the HID event system like a digitizer's
 * (IOHIDEventSystemClientDispatchEvent, entitlement com.apple.private.hid.client.event-dispatch: touch.entitlements),
 * as sweettouch does for the iOS 18 userland. arm64e + pacstrip.
 */
#include <CoreFoundation/CoreFoundation.h>
#include <dlfcn.h>
#include <mach/mach_time.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysctl.h>
#include <unistd.h>

typedef struct __IOHIDEvent *IOHIDEventRef;
typedef struct __IOHIDEventSystemClient *IOHIDEventSystemClientRef;
extern IOHIDEventSystemClientRef IOHIDEventSystemClientCreate(CFAllocatorRef);
extern void IOHIDEventSystemClientDispatchEvent(IOHIDEventSystemClientRef, IOHIDEventRef);
extern IOHIDEventRef IOHIDEventCreateDigitizerEvent(CFAllocatorRef, uint64_t ts, uint32_t transducer, uint32_t index,
    uint32_t identity, uint32_t eventMask, uint32_t buttonMask, double x, double y, double z, double pressure,
    double twist, bool range, bool touch, uint32_t options);
extern IOHIDEventRef IOHIDEventCreateDigitizerFingerEvent(CFAllocatorRef, uint64_t ts, uint32_t index, uint32_t identity,
    uint32_t eventMask, double x, double y, double z, double pressure, double twist, bool range, bool touch,
    uint32_t options);
extern IOHIDEventRef IOHIDEventCreateKeyboardEvent(CFAllocatorRef, uint64_t ts, uint32_t page, uint32_t usage,
    bool down, uint32_t flags);
extern void IOHIDEventAppendEvent(IOHIDEventRef parent, IOHIDEventRef child, uint32_t options);
extern void IOHIDEventSetIntegerValue(IOHIDEventRef, uint32_t field, int64_t value);
extern void IOHIDEventSetSenderID(IOHIDEventRef, uint64_t);

#define W 414.0
#define H 896.0
#define kIOHIDDigitizerTransducerTypeHand 3
#define kIOHIDDigitizerEventRange 0x1
#define kIOHIDDigitizerEventTouch 0x2
#define kIOHIDDigitizerEventPosition 0x4
#define kIOHIDEventFieldDigitizerIsDisplayIntegrated 0xb0019
#define SENDER 0x8000000817319372ULL            /* any fixed id: events are attributed to this "digitizer" */

static IOHIDEventSystemClientRef cli;

static void
finger(double x, double y, bool touch, uint32_t mask)
{
	/* a touch report for the clock boost, as the touch driver gives a finger's (kernel 1031; ignored without it) */
	int one = 1;
	if (touch)
		(void)sysctlbyname("kern.sweet_input_boost", NULL, NULL, &one, sizeof(one));
	uint64_t t = mach_absolute_time();
	IOHIDEventRef hand = IOHIDEventCreateDigitizerEvent(kCFAllocatorDefault, t, kIOHIDDigitizerTransducerTypeHand, 0, 0,
	    mask, 0, 0, 0, 0, 0, 0, touch, touch, 0);
	IOHIDEventSetIntegerValue(hand, kIOHIDEventFieldDigitizerIsDisplayIntegrated, 1);
	IOHIDEventRef f = IOHIDEventCreateDigitizerFingerEvent(kCFAllocatorDefault, t, 1, 2, mask, x / W, y / H, 0,
	    touch ? 1.0 : 0, 0, touch, touch, 0);
	IOHIDEventAppendEvent(hand, f, 0);
	IOHIDEventSetSenderID(hand, SENDER);
	IOHIDEventSystemClientDispatchEvent(cli, hand);
	CFRelease(f);
	CFRelease(hand);
}

/*
 * A wait that ends on time. usleep(8000) in an SSH shell comes back ~90 ms later here (timer coalescing for the shell's
 * QoS): a "300 ms" swipe took 3.4 s and gave SpringBoard touch moves at 11-20 Hz, and every page-swipe frame rate measured
 * with it was that of the input, not of the renderer (2026-11-24).
 */
static void
wait_us(uint64_t us)
{
	static mach_timebase_info_data_t tb;
	if (tb.denom == 0) {
		mach_timebase_info(&tb);
	}
	uint64_t end = mach_absolute_time() + us * 1000 * tb.denom / tb.numer;
	while (mach_absolute_time() < end) {
	}
}

static void
swipe(double x1, double y1, double x2, double y2, int ms)
{
	int steps = ms / 8 > 2 ? ms / 8 : 2;
	finger(x1, y1, true, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
	for (int i = 1; i <= steps; i++) {
		wait_us(8000);
		finger(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps, true, kIOHIDDigitizerEventPosition);
	}
	finger(x2, y2, false, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
}

/*
 * One finger down for SECONDS, moving left and right across Y without lifting (one sweep per MS), then up: the content under
 * it follows every frame, with no gap between gestures (a loop of `swipe` calls pauses between processes, which a per-second
 * frame meter counts as slow seconds).
 */
static void
drag(double x1, double x2, double y, int seconds, int ms, int step_ms)
{
	/* Positions follow the clock, not a step count, so a late step makes the finger jump instead of freezing it (the content
	 * stopping reads as slow frames). Run it through fgjob (a transient launchd job, ProcessType Interactive): everything
	 * dropbear starts is darwin-background, priority 4 with timers coalesced by up to 100 ms, so its steps come 0.1-1.5 s late
	 * (a busy wait instead gets preempted that long, and burns one of the four big cores the UI runs on). */
	mach_timebase_info_data_t tb;
	mach_timebase_info(&tb);
	uint64_t tick = (uint64_t)step_ms * 1000000ull * tb.denom / tb.numer, t0 = mach_absolute_time(), late = 0, inject = 0;
	double period = 2.0 * ms * 1e6, x = x1;
	/* Down 20 points outside the sweep, first move at X1: the pan starts there and the page sits where it started. The sweep
	 * then turns 8 points inside X1, because a page back at its exact start (a pixel over, after rounding) counts its
	 * neighbour as visible: from home page 1 that is the Today view, whose 9 widget stacks SpringBoard loads (and page 1's
	 * icon views it removes) in ~0.4 s, every sweep. */
	double dir = x2 > x1 ? 1 : -1;
	finger(x1 - 20 * dir, y, true, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
	wait_us(8000);
	finger(x1, y, true, kIOHIDDigitizerEventPosition);
	x1 += 8 * dir;
	for (uint64_t i = 1; i <= (uint64_t)seconds * 1000 / step_ms; i++) {
		mach_wait_until(t0 + i * tick);
		uint64_t now = mach_absolute_time(), over = (now - t0 - i * tick) * tb.numer / tb.denom;
		late = over > late ? over : late;
		double ph = fmod((double)((now - t0) * tb.numer / tb.denom), period) / period;
		x = x1 + (x2 - x1) * (ph <= 0.5 ? 2 * ph : 2 - 2 * ph);
		finger(x, y, true, kIOHIDDigitizerEventPosition);
		uint64_t d = (mach_absolute_time() - now) * tb.numer / tb.denom;
		inject = d > inject ? d : inject;
	}
	finger(x, y, false, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
	fprintf(stderr, "drag: %d steps, worst step %.1f ms late, slowest event dispatch %.1f ms\n", seconds * 1000 / step_ms, late / 1e6,
	        inject / 1e6);
}

static void
button(uint32_t page, uint32_t usage)
{
	for (int down = 1; down >= 0; down--) {
		IOHIDEventRef e = IOHIDEventCreateKeyboardEvent(kCFAllocatorDefault, mach_absolute_time(), page, usage, down, 0);
		IOHIDEventSetSenderID(e, SENDER);
		IOHIDEventSystemClientDispatchEvent(cli, e);
		CFRelease(e);
		usleep(80000);
	}
}

/* SpringBoardServices' SBSUndimScreen: the display comes on as for a touch, without a button's side effects */
static int
undim(void)
{
	static void (*fn)(void);
	if (!fn) {
		void *h = dlopen("/System/Library/PrivateFrameworks/SpringBoardServices.framework/SpringBoardServices", RTLD_LAZY);
		fn = h ? (void (*)(void))dlsym(h, "SBSUndimScreen") : NULL;
	}
	if (!fn)
		return -1;
	fn();
	return 0;
}

static void
wake(void)
{
	if (undim() != 0)
		button(0x0c, 0x40);   /* consumer page, menu */
}

int
main(int argc, char **argv)
{
	cli = IOHIDEventSystemClientCreate(kCFAllocatorDefault);
	if (!cli) {
		fprintf(stderr, "sweetwake: no HID event system client\n");
		return 1;
	}
	if (argc == 2 && !strcmp(argv[1], "wake")) {
		wake();
	} else if (argc == 2 && !strcmp(argv[1], "unlock")) {
		wake();
		usleep(600000);
		swipe(W / 2, H - 4, W / 2, H * 0.35, 250);   /* up from the bottom edge: lock screen -> home screen */
	} else if ((argc == 2 || argc == 3) && !strcmp(argv[1], "pages")) {
		int n = argc == 3 ? atoi(argv[2]) : 1;
		wake();
		for (int i = 0; i < n; i++) {
			usleep(500000);
			swipe(W * 0.85, H / 2, W * 0.15, H / 2, 250);   /* to the next page */
			usleep(1500000);
			swipe(W * 0.15, H / 2, W * 0.85, H / 2, 250);   /* and back */
		}
	} else if (argc == 3 && !strcmp(argv[1], "awake")) {
		int secs = atoi(argv[2]);
		if (fork() != 0)
			return 0;   /* the parent returns at once; the child keeps the display on */
		setsid();
		for (int t = 0; t <= secs; t += 10) {
			undim();
			sleep(10);
		}
		return 0;
	} else if (argc == 4 && !strcmp(argv[1], "tap")) {
		double x = atof(argv[2]), y = atof(argv[3]);
		finger(x, y, true, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
		usleep(60000);
		finger(x, y, false, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
	} else if ((argc == 6 || argc == 7) && !strcmp(argv[1], "swipe")) {
		swipe(atof(argv[2]), atof(argv[3]), atof(argv[4]), atof(argv[5]), argc == 7 ? atoi(argv[6]) : 300);
	} else if ((argc == 7 || argc == 8) && !strcmp(argv[1], "hold")) {
		double x1 = atof(argv[2]), y1 = atof(argv[3]), x2 = atof(argv[4]), y2 = atof(argv[5]);
		int hold = atoi(argv[6]), ms = argc == 8 ? atoi(argv[7]) : 600, steps = ms / 8 > 2 ? ms / 8 : 2;
		finger(x1, y1, true, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
		for (int i = 0; i < hold / 8; i++) {   /* a still finger keeps reporting, as a real one does */
			wait_us(8000);
			finger(x1, y1, true, kIOHIDDigitizerEventPosition);
		}
		for (int i = 1; i <= steps; i++) {
			wait_us(8000);
			finger(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps, true, kIOHIDDigitizerEventPosition);
		}
		for (int i = 0; i < 1200 / 8; i++) {   /* rest on the target until SpringBoard opens a folder there */
			wait_us(8000);
			finger(x2, y2, true, kIOHIDDigitizerEventPosition);
		}
		finger(x2, y2, false, kIOHIDDigitizerEventRange | kIOHIDDigitizerEventTouch);
	} else if (argc >= 6 && argc <= 8 && !strcmp(argv[1], "drag")) {
		drag(atof(argv[2]), atof(argv[3]), atof(argv[4]), atoi(argv[5]), argc >= 7 ? atoi(argv[6]) : 1000, argc == 8 ? atoi(argv[7]) : 8);
	} else if (argc == 2 && !strcmp(argv[1], "home")) {
		button(0x0c, 0x40);
	} else if (argc == 2 && !strcmp(argv[1], "lock")) {
		button(0x0c, 0x30);   /* consumer page, power */
	} else {
		fprintf(stderr, "usage: sweetwake wake | unlock | pages [N] | awake SECONDS | tap X Y | swipe X1 Y1 X2 Y2 [MS] | hold X1 Y1 X2 Y2 HOLD_MS [MS] | drag X1 X2 Y SECONDS [MS [STEP_MS]] | home | lock\n"
		                "       (points, 414x896)\n");
		return 2;
	}
	usleep(50000);
	return 0;
}
