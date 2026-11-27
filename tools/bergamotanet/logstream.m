/*
 * logstream PROC[,PROC]|- [SUBSTRING] [SECONDS]: live unified log (os_log) on the phone through LoggingSupport's
 * OSLogEventLiveStream: iOS has no `log` tool. Prints "pid process[subsystem] message" for the named processes
 * ("-" = all) whose message contains SUBSTRING (case-insensitive), for SECONDS (default 30).
 *   /var/root/logstream wifid,Preferences scan 60
 */
#import <Foundation/Foundation.h>
#include <dlfcn.h>
#include <objc/message.h>

@interface OSLogEventProxy : NSObject
@property (readonly) NSString *composedMessage, *process, *subsystem;
@property (readonly) int processIdentifier;
@end

int main(int argc, char **argv)
{
	@autoreleasepool {
		if (argc < 2) { fprintf(stderr, "usage: logstream PROC[,PROC]|- [SUBSTRING] [SECONDS]\n"); return 2; }
		if (!dlopen("/System/Library/PrivateFrameworks/LoggingSupport.framework/LoggingSupport", RTLD_NOW)) {
			fprintf(stderr, "logstream: %s\n", dlerror()); return 1;
		}
		NSSet *procs = strcmp(argv[1], "-") ? [NSSet setWithArray:[@(argv[1]) componentsSeparatedByString:@","]] : nil;
		NSString *needle = argc > 2 && argv[2][0] ? @(argv[2]) : nil;
		int secs = argc > 3 ? atoi(argv[3]) : 30;
		setvbuf(stdout, NULL, _IOLBF, 0);
		__block id stream = nil;
		id store = ((id (*)(Class, SEL))objc_msgSend)(NSClassFromString(@"OSLogEventLiveStore"), @selector(liveLocalStore));
		void (^ready)(id, NSError *) = ^(id source, NSError *err) {
			if (!source) { fprintf(stderr, "logstream: no live source: %s\n", err.description.UTF8String); exit(1); }
			stream = ((id (*)(id, SEL, id))objc_msgSend)([NSClassFromString(@"OSLogEventLiveStream") alloc],
			    @selector(initWithLiveSource:), source);
			((void (*)(id, SEL, unsigned long))objc_msgSend)(stream, @selector(setFlags:), 0x1 | 0x2); /* info + debug */
			((void (*)(id, SEL, id))objc_msgSend)(stream, @selector(setEventHandler:), ^(OSLogEventProxy *e) {
				NSString *m = e.composedMessage;
				if ((procs && ![procs containsObject:e.process]) || !m) return;
				if (needle && [m rangeOfString:needle options:NSCaseInsensitiveSearch].location == NSNotFound) return;
				printf("%d %s[%s] %s\n", e.processIdentifier, e.process.UTF8String, e.subsystem.UTF8String ?: "", m.UTF8String);
			});
			((void (*)(id, SEL, id))objc_msgSend)(stream, @selector(setInvalidationHandler:), ^(id s, long reason) {
				fprintf(stderr, "logstream: stream ended (%ld)\n", reason); exit(1);
			});
			((void (*)(id, SEL))objc_msgSend)(stream, @selector(activate));
		};
		((void (*)(id, SEL, id))objc_msgSend)(store, @selector(prepareWithCompletionHandler:), ready);
		CFRunLoopRunInMode(kCFRunLoopDefaultMode, secs, false);
	}
	return 0;
}
