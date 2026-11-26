// autolock [SECONDS|never]: read or set Auto-Lock (ManagedConfiguration setting "maxInactivity", what Settings writes)
#import <Foundation/Foundation.h>
#include <dlfcn.h>
#include <limits.h>

@interface MCProfileConnection : NSObject
+ (instancetype)sharedConnection;
- (id)effectiveValueForSetting:(NSString *)setting;
- (void)setValue:(id)value forSetting:(NSString *)setting;
@end

int main(int argc, char **argv)
{
	@autoreleasepool {
		if (!dlopen("/System/Library/PrivateFrameworks/ManagedConfiguration.framework/ManagedConfiguration", RTLD_NOW)) {
			fprintf(stderr, "autolock: no ManagedConfiguration: %s\n", dlerror());
			return 1;
		}
		Class c = NSClassFromString(@"MCProfileConnection");
		MCProfileConnection *mc = [c sharedConnection];
		if (argc == 2) {
			long v = strcmp(argv[1], "never") == 0 ? INT_MAX : atol(argv[1]);
			[mc setValue:@(v) forSetting:@"maxInactivity"];
		}
		printf("maxInactivity %s\n", [[[mc effectiveValueForSetting:@"maxInactivity"] description] UTF8String]);
	}
	return 0;
}
