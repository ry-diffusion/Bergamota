/*
 * bergamotawifijoin SSID [SECONDS]: join a Wi-Fi network through wifid (MobileWiFi), the way Settings does, then wait
 * for an IPv4 address on the Wi-Fi interface. The passphrase is read from stdin (one line; empty = open network), so it
 * never shows in a command line or in ps. iOS 18 Wi-Fi testing without hands (docs/WIFI_IOS26.md, kernel 1100-1130).
 *   printf '%s\n' "$PASS" | bergamotawifijoin MyNetwork 40
 * Exit 0 = associated and an address arrived; prints the scan hit (channel/RSSI), the join result and the address.
 */
#import <Foundation/Foundation.h>
#include <arpa/inet.h>
#include <dlfcn.h>
#include <ifaddrs.h>
#include <net/if.h>
#include <stdio.h>
#include <string.h>

typedef void *WiFiManagerClientRef, *WiFiDeviceClientRef, *WiFiNetworkRef;
/* MobileWiFi is private (no SDK stub): resolved with dlsym in main */
static WiFiManagerClientRef (*WiFiManagerClientCreate)(CFAllocatorRef, int);
static CFArrayRef (*WiFiManagerClientCopyDevices)(WiFiManagerClientRef);
static void (*WiFiManagerClientScheduleWithRunLoop)(WiFiManagerClientRef, CFRunLoopRef, CFStringRef);
static int (*WiFiManagerClientSetPower)(WiFiManagerClientRef, Boolean);
static CFStringRef (*WiFiDeviceClientGetInterfaceName)(WiFiDeviceClientRef);
static int (*WiFiDeviceClientScanAsync)(WiFiDeviceClientRef, CFDictionaryRef,
    void (*)(WiFiDeviceClientRef, CFArrayRef, intptr_t, void *), void *);
static int (*WiFiDeviceClientAssociateAsync)(WiFiDeviceClientRef, WiFiNetworkRef,
    void (*)(WiFiDeviceClientRef, WiFiNetworkRef, CFDictionaryRef, intptr_t, void *), void *);
static CFStringRef (*WiFiNetworkGetSSID)(WiFiNetworkRef);
static WiFiNetworkRef (*WiFiNetworkCreateCopy)(WiFiNetworkRef);
static void (*WiFiNetworkSetPassword)(WiFiNetworkRef, CFStringRef);

static int load_mobilewifi(void)
{
	void *h = dlopen("/System/Library/PrivateFrameworks/MobileWiFi.framework/MobileWiFi", RTLD_NOW);
	if (!h) { fprintf(stderr, "bergamotawifijoin: %s\n", dlerror()); return 0; }
#define L(f) if (!(*(void **)&f = dlsym(h, #f))) { fprintf(stderr, "bergamotawifijoin: no %s\n", #f); return 0; }
	L(WiFiManagerClientCreate) L(WiFiManagerClientCopyDevices) L(WiFiManagerClientScheduleWithRunLoop)
	L(WiFiManagerClientSetPower) L(WiFiDeviceClientGetInterfaceName) L(WiFiDeviceClientScanAsync)
	L(WiFiDeviceClientAssociateAsync) L(WiFiNetworkGetSSID) L(WiFiNetworkCreateCopy) L(WiFiNetworkSetPassword)
#undef L
	return 1;
}

static NSString *gSSID;
static WiFiNetworkRef gHit;
static intptr_t gScanErr = -1, gJoinErr = -1;

static void scanned(WiFiDeviceClientRef dev, CFArrayRef results, intptr_t err, void *ctx)
{
	(void)dev; (void)ctx;
	gScanErr = err;
	CFIndex n = results ? CFArrayGetCount(results) : 0;
	printf("bergamotawifijoin: scan: %ld networks (error %ld)\n", (long)n, (long)err);
	for (CFIndex i = 0; i < n; i++) {
		WiFiNetworkRef net = (WiFiNetworkRef)CFArrayGetValueAtIndex(results, i);
		NSString *ssid = (__bridge NSString *)WiFiNetworkGetSSID(net);
		if (ssid && [ssid isEqualToString:gSSID] && !gHit)
			gHit = WiFiNetworkCreateCopy(net);
	}
	CFRunLoopStop(CFRunLoopGetMain());
}

static void joined(WiFiDeviceClientRef dev, WiFiNetworkRef net, CFDictionaryRef info, intptr_t err, void *ctx)
{
	(void)dev; (void)net; (void)info; (void)ctx;
	gJoinErr = err;
	printf("bergamotawifijoin: join result %ld (0 = associated)\n", (long)err);
	CFRunLoopStop(CFRunLoopGetMain());
}

static int ipv4_of(const char *ifname, char *out, size_t len)
{
	struct ifaddrs *ifa, *p;
	int found = 0;
	if (getifaddrs(&ifa) != 0)
		return 0;
	for (p = ifa; p; p = p->ifa_next)
		if (p->ifa_addr && p->ifa_addr->sa_family == AF_INET && !strcmp(p->ifa_name, ifname)) {
			inet_ntop(AF_INET, &((struct sockaddr_in *)p->ifa_addr)->sin_addr, out, (socklen_t)len);
			found = 1;
			break;
		}
	freeifaddrs(ifa);
	return found;
}

int main(int argc, char **argv)
{
	@autoreleasepool {
		if (argc < 2) {
			fprintf(stderr, "usage: printf '%%s\\n' PASS | bergamotawifijoin SSID [SECONDS]\n");
			return 2;
		}
		gSSID = [NSString stringWithUTF8String:argv[1]];
		int secs = argc > 2 ? atoi(argv[2]) : 40;
		char pass[256] = "";
		if (!fgets(pass, sizeof pass, stdin))
			pass[0] = 0;
		pass[strcspn(pass, "\r\n")] = 0;

		if (!load_mobilewifi())
			return 1;
		WiFiManagerClientRef mgr = WiFiManagerClientCreate(kCFAllocatorDefault, 0);
		if (!mgr) { fprintf(stderr, "bergamotawifijoin: no WiFiManagerClient (wifid not running?)\n"); return 1; }
		WiFiManagerClientScheduleWithRunLoop(mgr, CFRunLoopGetMain(), kCFRunLoopDefaultMode);
		WiFiManagerClientSetPower(mgr, true);
		CFArrayRef devs = WiFiManagerClientCopyDevices(mgr);
		if (!devs || CFArrayGetCount(devs) == 0) { fprintf(stderr, "bergamotawifijoin: no Wi-Fi device\n"); return 1; }
		WiFiDeviceClientRef dev = (WiFiDeviceClientRef)CFArrayGetValueAtIndex(devs, 0);
		const char *ifname = [(__bridge NSString *)WiFiDeviceClientGetInterfaceName(dev) UTF8String] ?: "en0";
		printf("bergamotawifijoin: device %s\n", ifname);

		for (int tries = 0; tries < 3 && !gHit; tries++) {
			WiFiDeviceClientScanAsync(dev, (__bridge CFDictionaryRef)@{}, scanned, NULL);
			CFRunLoopRunInMode(kCFRunLoopDefaultMode, 20, false);
		}
		if (!gHit) { printf("bergamotawifijoin: \"%s\" not found in the scan\n", argv[1]); return 1; }
		if (pass[0])
			WiFiNetworkSetPassword(gHit, (__bridge CFStringRef)[NSString stringWithUTF8String:pass]);
		memset(pass, 0, sizeof pass);
		WiFiDeviceClientAssociateAsync(dev, gHit, joined, NULL);
		CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, false);
		if (gJoinErr != 0) { printf("bergamotawifijoin: join did not complete (%ld)\n", (long)gJoinErr); return 1; }
		char ip[64] = "";
		for (int t = 0; t < secs * 2 && !ipv4_of(ifname, ip, sizeof ip); t++)
			usleep(500000);
		printf("bergamotawifijoin: %s address %s\n", ifname, ip[0] ? ip : "(none yet)");
		return ip[0] ? 0 : 1;
	}
}
