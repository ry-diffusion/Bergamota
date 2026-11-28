/*
 * bergamotasbx: tell the kernel's sandbox stand-in (kernel 1157+) where every app extension's data container is, so
 * sandbox_container_path_for_pid answers for sandboxed extensions. RunningBoard launches them without a container in
 * the spawn attrs (observed with kernel 1156), PlugInKit then aborts each one ("plugin has invalid sandbox or container")
 * and chronod relaunches it every 1-2 s (People widget intents, 2026-11-28). For each PluginKitPlugin data container it
 * reads MCMMetadataIdentifier (the extension's bundle ID = its code-signing ID) and calls
 * sandbox_set_container_path_for_signing_id(id, path). The kernel keeps the table until reboot: run at every boot
 * (launchd job org.bergamota.sbx, root_tweaks18.bergamotasbx). Exit 0 also on an older kernel (nothing to do there).
 *   bergamotasbx [-v]
 */
#import <Foundation/Foundation.h>
#include <errno.h>
#include <string.h>

extern int sandbox_set_container_path_for_signing_id(const char *signing_id, const char *path);

int main(int argc, char **argv)
{
	@autoreleasepool {
		BOOL verbose = argc > 1 && !strcmp(argv[1], "-v");
		NSString *dir = @"/private/var/mobile/Containers/Data/PluginKitPlugin";
		int ok = 0, bad = 0;
		for (NSString *u in [[NSFileManager defaultManager] contentsOfDirectoryAtPath:dir error:nil]) {
			NSString *c = [dir stringByAppendingPathComponent:u];
			NSString *meta = [c stringByAppendingPathComponent:@".com.apple.mobile_container_manager.metadata.plist"];
			NSString *ident = [NSDictionary dictionaryWithContentsOfFile:meta][@"MCMMetadataIdentifier"];
			if (![ident isKindOfClass:[NSString class]] || !ident.length)
				continue;
			if (sandbox_set_container_path_for_signing_id(ident.UTF8String, c.UTF8String) == 0) {
				ok++;
				if (verbose) printf("%s %s\n", ident.UTF8String, c.UTF8String);
			} else {
				int e = errno;
				if (!bad++) printf("bergamotasbx: %s: %s\n", ident.UTF8String, strerror(e));
				if (e == ENOSPC) break;
			}
		}
		printf("bergamotasbx: %d extension containers registered, %d refused\n", ok, bad);
	}
	return 0;
}
