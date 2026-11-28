/*
 * sensorbridge.c: the phone's sensors for iOS 18, inside backboardd. A dylib linked into /usr/libexec/backboardd
 * (LC_LOAD_DYLIB, tools/renegade/root_tweaks18.py sensorbridge / --link-dylib) that reads /dev/bergamota-sensors
 * (xnu/patches-11215/1136) and turns the samples into the HID events iOS's own consumers read.
 *
 * Ported from Jesvi iOS / HakiOS (S20 port), userland/src/inject_sensors.c (MIT License, Copyright (c) 2025-2026 the
 * HakiOS authors; which in turn ports Scion's S9 inject.c), "redwood" renamed "bergamota". Changes for iOS 18 (details and
 * what is not verified: docs/SENSORS.md):
 *   - it is a self-starting dylib (constructor + thread), not two functions called from a GPU inject, and it makes its
 *     own IOHIDEventSystemClient (the S20 code used backboardd's touch input client; we have kernel HID touch)
 *   - the client is scheduled on a dispatch queue and activated, which iOS 18's virtual-service call requires
 *   - CMDeviceOrientationManager: the "Private" start/stop variants iOS 18 added are replaced too
 *   - the proximity-emitter ioctl and the S20's True Tone guard calls are gone; the auto-brightness thread is opt-in
 *     (file /private/var/tmp/sensorbridge.autobright)
 *   - the log goes to /private/var/tmp/sensorbridge.log; kill switch /private/var/tmp/sensorbridge.off
 *
 * The samples become events of built-in services on Apple's vendor page 0xFF00 (as on iOS 16.5; iOS 18.7.8 22H352
 * checked in the shared cache, docs/SENSORS.md):
 *   accelerometer  usage 0x03, type 13 events in g, iOS's axes (at rest the axis that points up reads -1 g): Android's
 *                  sign turned around
 *   ambient light  usage 0x04, type 12 events with the Illuminance field (0xC000B) in lux: CoreBrightness. Built in and
 *                  VendorID 0x5ac or it ignores the service; it asks the service for ALSCurveInfo
 *   proximity      usage 0x08 with Placement = 1, type 14, the Receiver bit (0x40) while something is near
 *   gyroscope      usage 0x09, type 20, degrees/s; magnetometer usage 0x0A, type 21, uT
 * Rotation: backboardd takes the orientation from CMDeviceOrientationManager; its methods are replaced and the
 * orientation is worked out from the accelerometer (gravity over 0.75 g on one axis, held for 300 ms).
 *
 * Boot-args sens_gyro_k= and sens_mag_k= (millionths: defaults 61 = 1/16400 dps and 54000 = 0.054 uT per count; the
 * S20's, unchecked here) and sens_als=0 (no light service) override without a rebuild.
 */
#include <Block.h>
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <dispatch/dispatch.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <mach/mach_time.h>
#include <math.h>
#include <objc/message.h>
#include <objc/runtime.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysctl.h>
#include <unistd.h>

#include "bergamota_sensors.h"
#include "brightness_policy.h"

typedef struct __IOHIDEvent *IOHIDEventRef;
typedef struct __IOHIDEventSystemClient *IOHIDEventSystemClientRef;
typedef struct __IOHIDServiceClient *IOHIDServiceClientRef;

struct virtual_service_callbacks {
	uint64_t version;               /* 2 */
	void (*notification)(void *target, void *refcon, IOHIDServiceClientRef service, uint32_t type, CFDictionaryRef property);
	Boolean (*setProperty)(void *target, void *refcon, IOHIDServiceClientRef service, CFStringRef key, CFTypeRef value);
	CFTypeRef (*copyProperty)(void *target, void *refcon, IOHIDServiceClientRef service, CFStringRef key);
	IOHIDEventRef (*copyEvent)(void *target, void *refcon, IOHIDServiceClientRef service, uint32_t type, IOHIDEventRef matching, uint32_t options);
	int (*setOutputEvent)(void *target, void *refcon, IOHIDServiceClientRef service, IOHIDEventRef event);
	CFTypeRef (*copyMatchingEvent)(void *target, void *refcon, IOHIDServiceClientRef service, CFDictionaryRef matching);
};

extern IOHIDServiceClientRef IOHIDVirtualServiceClientCreateWithCallbacks(IOHIDEventSystemClientRef client, CFDictionaryRef attributes,
    const struct virtual_service_callbacks *callbacks, void *target, void *refcon);
extern Boolean IOHIDVirtualServiceClientRemove(IOHIDServiceClientRef service);
extern Boolean IOHIDVirtualServiceClientDispatchEvent(IOHIDServiceClientRef service, IOHIDEventRef event);
extern IOHIDEventSystemClientRef IOHIDEventSystemClientCreate(CFAllocatorRef allocator);
extern void IOHIDEventSystemClientSetDispatchQueue(IOHIDEventSystemClientRef client, dispatch_queue_t queue);
extern void IOHIDEventSystemClientActivate(IOHIDEventSystemClientRef client);
extern void IOHIDEventSetIntegerValue(IOHIDEventRef event, uint32_t field, CFIndex value);
extern void IOHIDEventSetFloatValue(IOHIDEventRef event, uint32_t field, double value);
extern IOHIDEventRef IOHIDEventCreateAccelerometerEvent(CFAllocatorRef allocator, uint64_t time, double x, double y, double z,
    uint32_t options);
extern IOHIDEventRef IOHIDEventCreateGyroEvent(CFAllocatorRef allocator, uint64_t time, double x, double y, double z,
    uint32_t options);
extern IOHIDEventRef IOHIDEventCreateCompassEvent(CFAllocatorRef allocator, uint64_t time, double x, double y, double z,
    uint32_t options);
extern IOHIDEventRef IOHIDEventCreateAmbientLightSensorEvent(CFAllocatorRef allocator, uint64_t time, uint32_t level,
    uint32_t options);
extern IOHIDEventRef IOHIDEventCreateProximtyEvent(CFAllocatorRef allocator, uint64_t time, uint32_t mask, uint32_t options);

#define HID_PAGE_APPLE_VENDOR           0xff00
#define HID_USAGE_ACCELEROMETER         0x03
#define HID_USAGE_AMBIENT_LIGHT         0x04
#define HID_USAGE_PROXIMITY             0x08
#define HID_USAGE_GYRO                  0x09
#define HID_USAGE_COMPASS               0x0a
#define HID_EVENT_ALS                   12
#define HID_EVENT_ACCELEROMETER         13
#define HID_EVENT_PROXIMITY             14
#define HID_EVENT_GYRO                  20
#define HID_EVENT_COMPASS               21
#define FIELD_IS_BUILT_IN               0x4
#define FIELD_ALS_ILLUMINANCE           0xc000b
#define PROXIMITY_RECEIVER              0x40

static void
say(const char *format, ...)
{
	char line[256];
	va_list ap;
	int n, fd;

	va_start(ap, format);
	n = vsnprintf(line, sizeof(line), format, ap);
	va_end(ap);
	if (n > (int)sizeof(line) - 1) {
		n = sizeof(line) - 1;
	}
	write(2, line, (size_t)n);
	/* backboardd's stderr goes nowhere: keep the lines where the phone's SSH can read them */
	fd = open("/private/var/tmp/sensorbridge.log", O_WRONLY | O_CREAT | O_APPEND, 0644);
	if (fd >= 0) {
		write(fd, line, (size_t)n);
		close(fd);
	}
}

static CFNumberRef
number(int value)
{
	return CFNumberCreate(NULL, kCFNumberIntType, &value);
}

struct service {
	CFMutableDictionaryRef properties;
	int usage;
	volatile bool opened;
	IOHIDEventRef last;             /* the last event, for copyEvent (CoreBrightness asks for the light's) */
	uint32_t last_type;
};

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static int device = -1;
static double gyro_k = 1.0 / 16400;                     /* counts to HID degrees/s; CoreMotion converts once */
static double mag_k = 0.054;                            /* counts to uT */
/* ALS is on by default. sens_als=0 disables both the service and the automatic
 * luminance bridge. The controller null guard permits boot without a native ALS controller. */
static long use_als = 1;
static mach_timebase_info_data_t timebase;

static CFTypeRef
copy_property(__unused void *target, void *refcon, __unused IOHIDServiceClientRef service, CFStringRef key)
{
	CFTypeRef value = CFDictionaryGetValue(((struct service *)refcon)->properties, key);

	return value ? CFRetain(value) : NULL;
}

static Boolean
set_property(__unused void *target, void *refcon, __unused IOHIDServiceClientRef service, CFStringRef key, CFTypeRef value)
{
	static int told;
	struct service *made = refcon;
	if (made->usage == HID_USAGE_PROXIMITY && key && value && CFEqual(key, CFSTR("DetectionMode"))) {
		uint32_t mode = 0;      /* no emitter to switch on the Poco F1 (the S20 asked its hub through an ioctl): logged only */
		if (CFGetTypeID(value) == CFNumberGetTypeID() && CFNumberGetValue(value, kCFNumberSInt32Type, &mode))
			say("sensorbridge: proximity DetectionMode %u\n", mode);
	}

	if (key != NULL && value != NULL && CFEqual(key, CFSTR("DeviceOpenedByEventSystem")) &&
	    CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value)) {
		((struct service *)refcon)->opened = true;
	}
	if (key != NULL && told < 24) {
		char name[96] = "?";

		told++;
		CFStringGetCString(key, name, sizeof(name), kCFStringEncodingUTF8);
		say("sensorbridge: sensors: the event server sets \"%s\" on %p\n", name, refcon);
	}
	return true;
}

static void
notification(__unused void *target, __unused void *refcon, __unused IOHIDServiceClientRef service, __unused uint32_t type,
    __unused CFDictionaryRef property)
{
}

static IOHIDEventRef
copy_event(__unused void *target, void *refcon, __unused IOHIDServiceClientRef service, uint32_t type,
    __unused IOHIDEventRef matching, __unused uint32_t options)
{
	struct service *made = refcon;
	IOHIDEventRef event = NULL;

	pthread_mutex_lock(&lock);
	if (made != NULL && made->last != NULL && made->last_type == type) {
		event = (IOHIDEventRef)CFRetain(made->last);
	}
	pthread_mutex_unlock(&lock);
	return event;
}

static const struct virtual_service_callbacks callbacks = {
	.version = 2, .notification = notification, .setProperty = set_property, .copyProperty = copy_property,
	.copyEvent = copy_event,
};

/* a built-in service with one usage, made again until the event server opens it (inject.c make_service()'s reason) */
static IOHIDServiceClientRef
make_sensor(IOHIDEventSystemClientRef client, const char *product, int usage, bool top, struct service **made_out)
{
	struct service *made = calloc(1, sizeof(*made));
	IOHIDServiceClientRef service = NULL;
	CFMutableDictionaryRef properties = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks,
	    &kCFTypeDictionaryValueCallBacks);
	CFMutableDictionaryRef pair = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks,
	    &kCFTypeDictionaryValueCallBacks);
	CFStringRef name = CFStringCreateWithCString(NULL, product, kCFStringEncodingUTF8);
	CFNumberRef page_number = number(HID_PAGE_APPLE_VENDOR), usage_number = number(usage), zero = number(0);
	CFNumberRef one = number(1);
	CFArrayRef pairs;

	CFDictionarySetValue(pair, CFSTR("DeviceUsagePage"), page_number);
	CFDictionarySetValue(pair, CFSTR("DeviceUsage"), usage_number);
	pairs = CFArrayCreate(NULL, (const void **)&pair, 1, &kCFTypeArrayCallBacks);
	CFDictionarySetValue(properties, CFSTR("PrimaryUsagePage"), page_number);
	CFDictionarySetValue(properties, CFSTR("PrimaryUsage"), usage_number);
	CFDictionarySetValue(properties, CFSTR("DeviceUsagePage"), page_number);
	CFDictionarySetValue(properties, CFSTR("DeviceUsage"), usage_number);
	CFDictionarySetValue(properties, CFSTR("DeviceUsagePairs"), pairs);
	CFDictionarySetValue(properties, CFSTR("Built-In"), kCFBooleanTrue);
	CFDictionarySetValue(properties, CFSTR("Transport"), CFSTR("SPI"));
	CFDictionarySetValue(properties, CFSTR("Manufacturer"), CFSTR("bergamota"));
	CFDictionarySetValue(properties, CFSTR("Product"), name);
	CFDictionarySetValue(properties, CFSTR("VendorID"), zero);
	if (usage == HID_USAGE_AMBIENT_LIGHT) {
		/* CoreBrightness takes an ALS service only if it is built in AND its VendorID is Apple's (0x5ac: cache
		 * 0x1aeff9eb8 "subs w8, w8, #0x5ac"): without it its auto-brightness controller never started */
		CFNumberRef apple = number(0x5ac);

		CFDictionarySetValue(properties, CFSTR("VendorID"), apple);
		CFRelease(apple);
	}
	if (usage == HID_USAGE_AMBIENT_LIGHT) {
		/*
		 * Auto-brightness's lux -> nits curve (B, 2026-11-07): CoreBrightness asks the ALS service for ALSCurveInfo
		 * (-copyPropertyForKey:, cache 0x1af0369b0) and takes floats E0a E0b E1 E2 (lux) and L0a L0b L1 L2 (nits)
		 * and Prefs.thirdSlope; without it: "Failed to obtain ALS Curve info". The panel's own range: ~2-420 nits.
		 */
		static const char *keys[] = { "E0a", "E0b", "E1", "E2", "L0a", "L0b", "L1", "L2" };
		static const float values[] = { 0, 10, 150, 1500, 3, 30, 150, 420 };
		CFMutableDictionaryRef curve = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks,
		    &kCFTypeDictionaryValueCallBacks);
		CFMutableDictionaryRef prefs = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks,
		    &kCFTypeDictionaryValueCallBacks);
		float slope = 0;

		for (unsigned i = 0; i < sizeof(keys) / sizeof(keys[0]); i++) {
			CFStringRef k = CFStringCreateWithCString(NULL, keys[i], kCFStringEncodingUTF8);
			CFNumberRef v = CFNumberCreate(NULL, kCFNumberFloatType, &values[i]);

			CFDictionarySetValue(curve, k, v);
			CFRelease(k);
			CFRelease(v);
		}
		{
			CFNumberRef v = CFNumberCreate(NULL, kCFNumberFloatType, &slope);

			CFDictionarySetValue(prefs, CFSTR("thirdSlope"), v);
			CFRelease(v);
		}
		CFDictionarySetValue(curve, CFSTR("Prefs"), prefs);
		CFDictionarySetValue(properties, CFSTR("ALSCurveInfo"), curve);
		CFRelease(prefs);
		CFRelease(curve);
	}
	CFDictionarySetValue(properties, CFSTR("ProductID"), zero);
	if (top) {
		CFDictionarySetValue(properties, CFSTR("Placement"), one);
	}
	made->properties = properties;
	made->usage = usage;
	for (int attempt = 1; attempt <= 20; attempt++) {
		made->opened = false;
		service = IOHIDVirtualServiceClientCreateWithCallbacks(client, properties, &callbacks, NULL, made);
		if (service == NULL) {
			break;
		}
		for (int tenths = 0; tenths < 20 && !made->opened; tenths++) {
			usleep(100000);
		}
		if (made->opened) {
			break;
		}
		say("sensorbridge: sensors: the event server has not opened \"%s\" (attempt %d): making it again\n", product, attempt);
		IOHIDVirtualServiceClientRemove(service);
		CFRelease(service);
		service = NULL;
	}
	CFRelease(one);
	*made_out = made;
	return service;
}

static struct {
	IOHIDServiceClientRef accel, light, proximity, gyro, compass;
	struct service *accel_made, *light_made, *proximity_made, *gyro_made, *compass_made;
} sensor;

static void
push_event(IOHIDServiceClientRef service, struct service *made, IOHIDEventRef event, uint32_t type)
{
	IOHIDEventSetIntegerValue(event, FIELD_IS_BUILT_IN, 1);
	IOHIDVirtualServiceClientDispatchEvent(service, event);
	pthread_mutex_lock(&lock);
	if (made->last != NULL) {
		CFRelease(made->last);
	}
	made->last = (IOHIDEventRef)CFRetain(event);
	made->last_type = type;
	pthread_mutex_unlock(&lock);
	CFRelease(event);
}

/* ---- the orientation, for backboardd through CMDeviceOrientationManager ---------------------------------------- */

static volatile int orientation;                /* UIDeviceOrientation: 0 unknown, 1-6 */
static id orientation_queue;
static void (^orientation_handler)(id, id);

static id
make_orientation(int value)
{
	Class cls = objc_getClass("CMDeviceOrientation");
	id object = cls ? ((id (*)(id, SEL))objc_msgSend)((id)cls, sel_registerName("alloc")) : nil;

	if (object == nil) {
		return nil;
	}
	return ((id (*)(id, SEL, int, double))objc_msgSend)(object, sel_registerName("initWithOrientation:andTimestamp:"), value,
	    CFAbsoluteTimeGetCurrent());
}

static void
orientation_changed(int value)
{
	id queue;
	void (^handler)(id, id);

	pthread_mutex_lock(&lock);
	orientation = value;
	queue = orientation_queue;
	handler = orientation_handler ? Block_copy(orientation_handler) : NULL;
	pthread_mutex_unlock(&lock);
	say("sensorbridge: sensors: orientation %d (%s)\n", value, value == 1 ? "portrait" : value == 2 ? "upside down" :
	    value == 3 ? "home button right" : value == 4 ? "home button left" : value == 5 ? "face up" :
	    value == 6 ? "face down" : "unknown");
	if (handler == NULL || queue == nil) {
		if (handler != NULL) {
			Block_release(handler);
		}
		return;
	}
	((void (*)(id, SEL, void (^)(void)))objc_msgSend)(queue, sel_registerName("addOperationWithBlock:"), ^{
		id o = make_orientation(value);

		handler(o, nil);
		if (o != nil) {
			((void (*)(id, SEL))objc_msgSend)(o, sel_registerName("release"));
		}
		Block_release(handler);
	});
}

static void
my_start_orientation(__unused id self, __unused SEL selector, id queue, void (^handler)(id, id))
{
	int now;

	pthread_mutex_lock(&lock);
	if (orientation_handler != NULL) {
		Block_release(orientation_handler);
	}
	if (orientation_queue != nil) {
		((void (*)(id, SEL))objc_msgSend)(orientation_queue, sel_registerName("release"));
	}
	orientation_handler = handler ? Block_copy(handler) : NULL;
	orientation_queue = queue ? ((id (*)(id, SEL))objc_msgSend)(queue, sel_registerName("retain")) : nil;
	now = orientation;
	pthread_mutex_unlock(&lock);
	say("sensorbridge: sensors: backboardd asks for the orientation (now %d)\n", now);
	if (now != 0) {
		orientation_changed(now);
	}
}

static void
my_stop_orientation(__unused id self, __unused SEL selector)
{
	pthread_mutex_lock(&lock);
	if (orientation_handler != NULL) {
		Block_release(orientation_handler);
		orientation_handler = NULL;
	}
	if (orientation_queue != nil) {
		((void (*)(id, SEL))objc_msgSend)(orientation_queue, sel_registerName("release"));
		orientation_queue = nil;
	}
	pthread_mutex_unlock(&lock);
}

static id
my_orientation_blocking(__unused id self, __unused SEL selector)
{
	id o = make_orientation(orientation);

	return o ? ((id (*)(id, SEL))objc_msgSend)(o, sel_registerName("autorelease")) : nil;
}

static BOOL
my_orientation_available(__unused id self, __unused SEL selector)
{
	return YES;
}

static void
orientation_fixes(void)
{
	static const struct { const char *name; IMP imp; } methods[] = {
		{ "startDeviceOrientationUpdatesToQueue:withHandler:", (IMP)my_start_orientation },
		{ "startDeviceOrientationUpdatesPrivateToQueue:withHandler:", (IMP)my_start_orientation },      /* iOS 18 has both */
		{ "stopDeviceOrientationUpdates", (IMP)my_stop_orientation },
		{ "stopDeviceOrientationUpdatesPrivate", (IMP)my_stop_orientation },
		{ "deviceOrientationBlocking", (IMP)my_orientation_blocking },
		{ "isDeviceOrientationAvailable", (IMP)my_orientation_available },
	};
	Class cls = objc_getClass("CMDeviceOrientationManager");
	unsigned replaced = 0;

	if (cls == NULL) {
		say("sensorbridge: sensors: no CMDeviceOrientationManager here: the orientation is not taken over\n");
		return;
	}
	for (unsigned i = 0; i < sizeof(methods) / sizeof(methods[0]); i++) {
		Method method = class_getInstanceMethod(cls, sel_registerName(methods[i].name));

		if (method != NULL) {
			method_setImplementation(method, methods[i].imp);
			replaced++;
		}
	}
	say("sensorbridge: %u of CMDeviceOrientationManager's methods replaced\n", replaced);
}

/* the orientation from a sample (g, iOS's axes), filtered; a change is said after it held for 300 ms */
static void
orientation_from(double x, double y, double z, uint64_t now_us)
{
	static double fx, fy, fz;
	static int candidate;
	static uint64_t since_us;
	static bool primed;
	double ax, ay, az;
	int o = 0;

	if (!primed) {
		fx = x, fy = y, fz = z;
		primed = true;
	}
	fx += 0.2 * (x - fx);
	fy += 0.2 * (y - fy);
	fz += 0.2 * (z - fz);
	ax = fabs(fx);
	ay = fabs(fy);
	az = fabs(fz);
	if (ay >= ax && ay >= az && ay > 0.75) {
		o = fy < 0 ? 1 : 2;
	} else if (ax >= ay && ax >= az && ax > 0.75) {
		o = fx < 0 ? 3 : 4;
	} else if (az > 0.75) {
		o = fz < 0 ? 5 : 6;
	}
	if (o == 0) {
		return;
	}
	if (o != candidate) {
		candidate = o;
		since_us = now_us;
		return;
	}
	if (o != orientation && now_us - since_us >= 300000) {
		orientation_changed(o);
	}
}

/* ---- the samples ------------------------------------------------------------------------------------------------ */

static uint64_t
micros(uint64_t absolute)
{
	return absolute * timebase.numer / timebase.denom / 1000;
}

/* Port-side automatic luminance bridge. The stock controller receives TrustedLux
 * but its automatic ramp does not reach the backlight on this platform. Keep
 * Apple's setting, display-off state and DisplayNitsKey output API; only the
 * ambient-to-target policy and smoothing live here. No True Tone spoofing. */
static double automatic_lux;
static uint64_t automatic_sample_at;

static CFTypeRef
automatic_property(id client, CFStringRef key)
{
	return ((CFTypeRef (*)(id, SEL, CFStringRef))objc_msgSend)(client,
	    sel_registerName("copyPropertyForKey:"), key);
}

static bool
automatic_boolean(id client, CFStringRef key)
{
	CFTypeRef value = automatic_property(client, key);
	bool result = false;
	if (value != NULL) {
		if (CFGetTypeID(value) == CFBooleanGetTypeID()) result = CFBooleanGetValue(value);
		else if (CFGetTypeID(value) == CFNumberGetTypeID()) {
			int n = 0;
			if (CFNumberGetValue(value, kCFNumberIntType, &n)) result = n != 0;
		}
		CFRelease(value);
	}
	return result;
}

static double
automatic_current(id client)
{
	CFTypeRef value = automatic_property(client, CFSTR("DisplayBrightness"));
	double nits = -1;
	if (value != NULL) {
		if (CFGetTypeID(value) == CFDictionaryGetTypeID()) {
			CFTypeRef n = CFDictionaryGetValue(value, CFSTR("Nits"));
			if (n != NULL && CFGetTypeID(n) == CFNumberGetTypeID())
				CFNumberGetValue(n, kCFNumberDoubleType, &nits);
		}
		CFRelease(value);
	}
	return nits;
}

/* The Control Center slider also writes the legacy backlight interface.
 * Observe that input separately: DisplayBrightness.Nits need not change when
 * backboardd sends a legacy slider write. Native nits writes leave this value
 * unchanged, so this distinguishes a manual gesture from our own output. */
static double
automatic_slider(void)
{
	static io_service_t backlight;
	if (backlight == 0) backlight = IOServiceGetMatchingService(0, IOServiceMatching("AppleARMBacklight"));
	if (backlight == 0) return -1;
	CFTypeRef value = IORegistryEntryCreateCFProperty(backlight, CFSTR("IODisplayParameters"), NULL, 0);
	double level = -1, maximum = 0;
	if (value != NULL && CFGetTypeID(value) == CFDictionaryGetTypeID()) {
		CFTypeRef brightness = CFDictionaryGetValue(value, CFSTR("brightness"));
		if (brightness != NULL && CFGetTypeID(brightness) == CFDictionaryGetTypeID()) {
			CFTypeRef n = CFDictionaryGetValue(brightness, CFSTR("value"));
			CFTypeRef m = CFDictionaryGetValue(brightness, CFSTR("max"));
			if (n != NULL && m != NULL && CFGetTypeID(n) == CFNumberGetTypeID() && CFGetTypeID(m) == CFNumberGetTypeID()) {
				CFNumberGetValue(n, kCFNumberDoubleType, &level);
				CFNumberGetValue(m, kCFNumberDoubleType, &maximum);
			}
		}
	}
	if (value != NULL) CFRelease(value);
	return maximum > 0 && level >= 0 ? level / maximum : -1;
}

static void *
automatic_brightness_thread(__unused void *arg)
{
	id client = NULL;
	uint64_t gate_at = 0, step_at = 0, report_at = 0, save_at = 0;
	useconds_t wait_us = 500000;
	struct rb_policy policy = {0};
	bool loaded = false;
	double current = 3, sent = -1, last_slider = -1;
	bool active = false, enabled = false;
	pthread_setname_np("bergamota auto brightness");
	for (;;) {
		usleep(wait_us);
		wait_us = 500000;
		id pool = ((id (*)(id, SEL))objc_msgSend)(
		    ((id (*)(Class, SEL))objc_msgSend)(objc_getClass("NSAutoreleasePool"), sel_registerName("alloc")),
		    sel_registerName("init"));
		if (client == NULL) {
			Class cls = objc_getClass("BrightnessSystemClient");
			if (cls != NULL) client = ((id (*)(id, SEL))objc_msgSend)(
			    ((id (*)(Class, SEL))objc_msgSend)(cls, sel_registerName("alloc")), sel_registerName("init"));
		}
		if (!loaded) {
			CFPropertyListRef saved = CFPreferencesCopyAppValue(CFSTR("BrightnessAnchor"), CFSTR("org.bergamota.display"));
			if (saved != NULL && CFGetTypeID(saved) == CFDictionaryGetTypeID()) {
				CFTypeRef l = CFDictionaryGetValue(saved, CFSTR("Lux")), n = CFDictionaryGetValue(saved, CFSTR("Nits"));
				if (l && n && CFGetTypeID(l) == CFNumberGetTypeID() && CFGetTypeID(n) == CFNumberGetTypeID()) {
					CFNumberGetValue(l, kCFNumberDoubleType, &policy.anchor_lux);
					CFNumberGetValue(n, kCFNumberDoubleType, &policy.anchor_nits);
					policy.anchored = isfinite(policy.anchor_lux) && policy.anchor_lux >= 0 &&
					    isfinite(policy.anchor_nits) && policy.anchor_nits >= 3 && policy.anchor_nits <= 420;
				}
			}
			if (saved) CFRelease(saved);
			loaded = true;
		}
		uint64_t now = micros(mach_absolute_time()), sample_at;
		double lux;
		pthread_mutex_lock(&lock);
		lux = automatic_lux;
		sample_at = automatic_sample_at;
		pthread_mutex_unlock(&lock);
		if (save_at && now >= save_at && policy.anchored) {
			CFNumberRef l = CFNumberCreate(NULL, kCFNumberDoubleType, &policy.anchor_lux);
			CFNumberRef n = CFNumberCreate(NULL, kCFNumberDoubleType, &policy.anchor_nits);
			const void *keys[] = { CFSTR("Lux"), CFSTR("Nits") }, *values[] = { l, n };
			if (l && n) {
				CFDictionaryRef saved = CFDictionaryCreate(NULL, keys, values, 2,
				    &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
				CFPreferencesSetAppValue(CFSTR("BrightnessAnchor"), saved, CFSTR("org.bergamota.display"));
				CFPreferencesAppSynchronize(CFSTR("org.bergamota.display"));
				CFRelease(saved);
			}
			if (l) CFRelease(l); if (n) CFRelease(n);
			save_at = 0;
		}
		if (client == NULL || sample_at == 0 || now < sample_at || now - sample_at > 3000000) {
			active = false;
			wait_us = 1000000;
			goto next;
		}
		if (now - gate_at >= 250000) {
			gate_at = now;
			enabled = automatic_boolean(client, CFSTR("DisplayBrightnessAuto"));
			bool on = automatic_boolean(client, CFSTR("DisplayOn"));
			if (!enabled || !on) {
				active = false;
				sent = -1;
				wait_us = 1000000;
				goto next;
			}
			double actual = automatic_current(client);
			double slider = automatic_slider();
			if (!active) {
				if (!isfinite(actual) || actual < 2 || actual > 420) goto next;
				current = actual;
				sent = -1;
				step_at = now;
				active = true;
				last_slider = slider;
				rb_wake(&policy, now / 1e6, actual);
				say("sensorbridge: auto brightness: active at %.2f lux, %.2f nits\n", lux, current);
			} else if (slider >= 0 && last_slider >= 0 && fabs(slider - last_slider) > .0001) {
				actual = fmax(3, fmin(420, rb_slider_nits(slider)));   /* the kernel's slider curve */
				/* Anchor a manual adjustment to this lighting; the policy holds it
				 * locally and releases it across a sustained large ambient change. */
				rb_manual(&policy, now / 1e6, lux, actual);
				save_at = now + 10000000;
				current = actual;
				sent = actual;

			}
			last_slider = slider;
		}

		if (!active || !enabled) goto next;
		{
			double target = rb_observe(&policy, now / 1e6, lux);
			double dt = fmin(.5, (now - step_at) / 1e6);
			step_at = now;
			double tau = target > current ? .5 : 1.2;    /* ramp time constants, s (were .9 / 2) */
			if (fabs(target - current) > fmax(.2, current * .01)) wait_us = 50000;
			current = exp(log(current) + (log(target) - log(current)) * (1 - exp(-dt / tau)));
			if (fabs(current - target) < .15) current = target;
			if (sent >= 0 && fabs(current - sent) < fmax(.15, sent * .005)) goto next;
			/* Check display power again immediately before the write. */
			if (!automatic_boolean(client, CFSTR("DisplayOn")) ||
			    !automatic_boolean(client, CFSTR("DisplayBrightnessAuto"))) { active = false; goto next; }
			CFNumberRef number = CFNumberCreate(NULL, kCFNumberDoubleType, &current);
			const void *keys[] = { CFSTR("Brightness"), CFSTR("Commit") };
			const void *values[] = { number, kCFBooleanFalse };
			CFDictionaryRef request = number ? CFDictionaryCreate(NULL, keys, values, 2,
			    &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks) : NULL;
			if (request != NULL) {
				bool ok = ((bool (*)(id, SEL, CFTypeRef, CFStringRef))objc_msgSend)(client,
				    sel_registerName("setProperty:forKey:"), request, CFSTR("DisplayNitsKey"));
				if (ok) sent = current;
				CFRelease(request);
			}
			if (number != NULL) CFRelease(number);
			if (now - report_at > 5000000) {
				report_at = now;
				say("sensorbridge: auto brightness: %.2f lux -> %.2f nits (target %.2f, anchor %s)\n", lux, sent, target, policy.anchored ? "yes" : "no");
			}
		}
next:
		((void (*)(id, SEL))objc_msgSend)(pool, sel_registerName("drain"));
	}
	return NULL;
}

/*
 * iOS 18: IOHIDVirtualServiceClientCreateWithCallbacks refuses a client that is not scheduled ("Client must be scheduled
 * with dispatch queue or runloop", IOKit 0x18b75e5c4), and this dylib has no touch client of backboardd's to borrow, so
 * it makes its own: Create, SetDispatchQueue, Activate. The server side (backboardd itself, entitlement
 * com.apple.iohideventsystem.server) lets a connection create a virtual service when it is an Admin type or its process
 * has com.apple.private.hid.client.event-dispatch, which backboardd has; the client type is not checked.
 */
static IOHIDEventSystemClientRef
make_client(void)
{
	IOHIDEventSystemClientRef client = IOHIDEventSystemClientCreate(kCFAllocatorDefault);
	dispatch_queue_t queue = dispatch_queue_create("org.bergamota.sensorbridge.hid", DISPATCH_QUEUE_SERIAL);

	if (client == NULL) {
		return NULL;
	}
	IOHIDEventSystemClientSetDispatchQueue(client, queue);
	IOHIDEventSystemClientActivate(client);
	return client;
}

static void *
sensors_thread(__unused void *arg)
{
	IOHIDEventSystemClientRef client = NULL;
	struct bergamota_sensor_sample s;
	uint64_t count[8] = { 0 }, front_at = 0, lux_at = 0;
	uint32_t lux_sent = UINT32_MAX;
	int near = -1;

	pthread_setname_np("bergamota sensors");
	sleep(3);       /* backboardd's event server is not up while its constructors run */
	/* ambient light first, for CoreBrightness's start; the accelerometer doubles as the "is the server there yet" probe */
	for (int attempt = 1; attempt <= 100; attempt++) {
		client = make_client();
		if (client != NULL) {
			sensor.light = use_als ? make_sensor(client, "Bergamota ambient light sensor", HID_USAGE_AMBIENT_LIGHT, false,
			    &sensor.light_made) : NULL;
			sensor.accel = make_sensor(client, "Bergamota accelerometer", HID_USAGE_ACCELEROMETER, false, &sensor.accel_made);
			if (sensor.accel != NULL) {
				break;
			}
			say("sensorbridge: attempt %d: no virtual service yet (client %p)\n", attempt, (void *)client);
		} else {
			say("sensorbridge: attempt %d: IOHIDEventSystemClientCreate returned NULL\n", attempt);
		}
		sleep(3);
	}
	if (sensor.accel == NULL) {
		say("sensorbridge: giving up\n");
		return NULL;
	}
	sensor.proximity = make_sensor(client, "Bergamota proximity sensor", HID_USAGE_PROXIMITY, true, &sensor.proximity_made);
	sensor.gyro = make_sensor(client, "Bergamota gyroscope", HID_USAGE_GYRO, false, &sensor.gyro_made);
	sensor.compass = make_sensor(client, "Bergamota magnetometer", HID_USAGE_COMPASS, false, &sensor.compass_made);
	say("sensorbridge: sensors: services: light %p, accelerometer %p, proximity %p, gyroscope %p, magnetometer %p\n",
	    (void *)sensor.light, (void *)sensor.accel, (void *)sensor.proximity, (void *)sensor.gyro, (void *)sensor.compass);
	if (sensor.light != NULL && access("/private/var/tmp/sensorbridge.autobright", F_OK) == 0) {   /* opt-in, see docs/SENSORS.md */
		pthread_t automatic_thread;
		if (pthread_create(&automatic_thread, NULL, automatic_brightness_thread, NULL) == 0)
			pthread_detach(automatic_thread);
	}
	while (read(device, &s, sizeof(s)) == (ssize_t)sizeof(s)) {
		uint64_t now_us = micros(s.time);

		if (s.type < 8 && count[s.type]++ == 0) {
			say("sensorbridge: sensors: the first sample of type %u: %d %d %d %d (scale %d)\n", s.type, s.v[0], s.v[1], s.v[2],
			    s.v[3], s.scale);
		}
		switch (s.type) {
		case BERGAMOTA_SENSOR_ACCEL: {
			double k = (s.scale > 0 ? s.scale : 8) / 32768.0;
			double x = -s.v[0] * k, y = -s.v[1] * k, z = -s.v[2] * k;      /* Android's sign to iOS's */

			orientation_from(x, y, z, now_us);
			if (sensor.accel != NULL) {
				push_event(sensor.accel, sensor.accel_made, IOHIDEventCreateAccelerometerEvent(NULL, s.time, x, y, z, 0),
				    HID_EVENT_ACCELEROMETER);
			}
			break;
		}
		case BERGAMOTA_SENSOR_GYRO:
			if (sensor.gyro != NULL) {
				push_event(sensor.gyro, sensor.gyro_made, IOHIDEventCreateGyroEvent(NULL, s.time, s.v[0] * gyro_k,
				    s.v[1] * gyro_k, s.v[2] * gyro_k, 0), HID_EVENT_GYRO);
			}
			break;
		case BERGAMOTA_SENSOR_MAG:
			if (sensor.compass != NULL) {
				push_event(sensor.compass, sensor.compass_made, IOHIDEventCreateCompassEvent(NULL, s.time, s.v[0] * mag_k,
				    s.v[1] * mag_k, s.v[2] * mag_k, 0), HID_EVENT_COMPASS);
			}
			break;
		case BERGAMOTA_SENSOR_LIGHT: {
			uint32_t lux = (uint32_t)(s.v[0] < 0 ? 0 : s.v[0]);
			uint32_t diff;

			if (s.v[2] == BERGAMOTA_LIGHT_FRONT) {
				front_at = now_us;
			} else if (front_at != 0 && now_us - front_at < 5000000) {
				break;          /* the front sensor speaks: the rear one is not what the screen sees */
			}
			pthread_mutex_lock(&lock);
			automatic_lux = lux;
			automatic_sample_at = now_us;
			pthread_mutex_unlock(&lock);
			diff = lux > lux_sent ? lux - lux_sent : lux_sent - lux;
			/* on a change of more than 5 % (or 2 lux), and once a second anyway */
			if (sensor.light != NULL && (lux_sent == UINT32_MAX || diff * 20 > lux_sent || diff > 2 ||
			    now_us - lux_at > 1000000)) {
				IOHIDEventRef event = IOHIDEventCreateAmbientLightSensorEvent(NULL, s.time, lux, 0);

				IOHIDEventSetFloatValue(event, FIELD_ALS_ILLUMINANCE, (double)lux);
				push_event(sensor.light, sensor.light_made, event, HID_EVENT_ALS);
				lux_sent = lux;
				lux_at = now_us;
			}
			break;
		}
		case BERGAMOTA_SENSOR_PROXIMITY:
			if (sensor.proximity != NULL && (s.v[0] != 0) != near) {
				near = s.v[0] != 0;
				say("sensorbridge: sensors: proximity %s\n", near ? "near" : "far");
				push_event(sensor.proximity, sensor.proximity_made, IOHIDEventCreateProximtyEvent(NULL, s.time, near ?
				    PROXIMITY_RECEIVER : 0, 0), HID_EVENT_PROXIMITY);
			}
			break;
		default:
			break;          /* the barometer: no HID consumer of ours yet (NOTES.md) */
		}
	}
	say("sensorbridge: sensors: %s: %s\n", BERGAMOTA_SENSORS_DEVICE, strerror(errno));
	return NULL;
}


static long
boot_arg(const char *arguments, const char *name, long fallback)
{
	size_t n = strlen(name);

	for (const char *a = arguments; (a = strstr(a, name)) != NULL; a += n) {
		if ((a == arguments || a[-1] == ' ') && a[n] == '=') {
			return strtol(a + n + 1, NULL, 0);
		}
	}
	return fallback;
}

/*
 * Runs when backboardd loads the dylib (an LC_LOAD_DYLIB added by tools/renegade root_tweaks18.sensorbridge): before
 * backboardd asks CoreMotion for the orientation. Inert unless /dev/bergamota-sensors exists (kernel boot-arg
 * bergamotasensors=1) and /private/var/tmp/sensorbridge.off does not.
 */
__attribute__((constructor)) static void
sensorbridge_init(void)
{
	char arguments[1024] = "";
	size_t size = sizeof(arguments) - 1;
	pthread_t thread;

	if (strcmp(getprogname(), "backboardd") != 0 || access("/private/var/tmp/sensorbridge.off", F_OK) == 0) {
		return;
	}
	device = open(BERGAMOTA_SENSORS_DEVICE, O_RDONLY);
	if (device < 0) {
		return;                 /* no sensor device in the kernel: nothing of this happens */
	}
	mach_timebase_info(&timebase);
	if (sysctlbyname("kern.bootargs", arguments, &size, NULL, 0) == 0) {
		gyro_k = boot_arg(arguments, "sens_gyro_k", 61) / 1e6;
		mag_k = boot_arg(arguments, "sens_mag_k", 54000) / 1e6;
		use_als = boot_arg(arguments, "sens_als", 1);
	}
	say("sensorbridge: %s open, bridge starting\n", BERGAMOTA_SENSORS_DEVICE);
	orientation_fixes();
	if (pthread_create(&thread, NULL, sensors_thread, NULL) == 0) {
		pthread_detach(thread);
	}
}
