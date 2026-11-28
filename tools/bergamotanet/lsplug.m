// lsplug /Applications/X.app...: register a system app with its embedded extensions (PlugIns/*.appex) in LaunchServices,
// the way uicache does (registerApplicationDictionary: with _LSBundlePlugins). For system apps whose extensions LS never took:
// on the iOS 26 root every /Applications app had 0 plugins, so chronod found no widget/control providers (home widget "Waiting...",
// Control Center layout regenerated empty, controls gallery would not open). registerApplication: / registerPlugin: did not help.
// Sandboxed extensions (com.apple.security.app-sandbox) get their PluginKitPlugin data container and HOME/TMPDIR, as uicache
// does: without them LS has dataContainerURL nil and an NSExtension aborts in PlugInKit ("plugin has invalid sandbox or
// container"); chronod relaunched the People widget's intents extension every ~1.25 s on 2026-11-28 (crash + ReportCrash loop).
// Idempotent: an app without extensions, or whose extensions are all registered (with their containers) already, is skipped.
// Exit status 1 when an app's registration failed or none of its extensions shows up afterwards (lsd not up yet): run it again.
// Built by tools/bergamotanet/build.sh (research/bergamotanet/ios18/lsplug); run at first boot by root_tweaks18.firstboot (renegade).
// By hand on the phone (root): lsplug /Applications/*.app
#import <Foundation/Foundation.h>
#import <objc/message.h>
// Security's static-code API (private on iOS, what uicache uses to read entitlements)
typedef struct __SecCode *SecStaticCodeRef;
extern OSStatus SecStaticCodeCreateWithPathAndAttributes(CFURLRef, uint32_t, CFDictionaryRef, SecStaticCodeRef *);
extern OSStatus SecCodeCopySigningInformation(SecStaticCodeRef, uint32_t, CFDictionaryRef *);
static BOOL sandboxed(NSString *bundle) {
  SecStaticCodeRef code = NULL; CFDictionaryRef info = NULL; BOOL r = NO;
  if (SecStaticCodeCreateWithPathAndAttributes((__bridge CFURLRef)[NSURL fileURLWithPath:bundle], 0, NULL, &code) || !code) return NO;
  if (!SecCodeCopySigningInformation(code, 1 << 2 /* kSecCSSigningInformation */, &info) && info) {
    NSDictionary *ents = ((__bridge NSDictionary *)info)[@"entitlements-dict"];
    r = [ents[@"com.apple.security.app-sandbox"] boolValue];
    CFRelease(info);
  }
  CFRelease(code);
  return r;
}
// the extension's existing PluginKitPlugin data container (lsd made them; MobileContainerManager refuses root, error 98)
static NSString *container(NSString *pid) {
  static NSMutableDictionary *byId;
  if (!byId) {
    byId = [NSMutableDictionary dictionary];
    NSString *dir = @"/private/var/mobile/Containers/Data/PluginKitPlugin";
    for (NSString *u in [[NSFileManager defaultManager] contentsOfDirectoryAtPath:dir error:nil]) {
      NSString *c = [dir stringByAppendingPathComponent:u];
      NSString *i = [NSDictionary dictionaryWithContentsOfFile:[c stringByAppendingPathComponent:@".com.apple.mobile_container_manager.metadata.plist"]][@"MCMMetadataIdentifier"];
      if (i) byId[i] = c;
    }
  }
  if (!byId[pid]) printf("  %s: no data container\n", pid.UTF8String);
  return byId[pid];
}
static NSMutableDictionary *base(NSString *ident, NSString *path, NSString *type) {
  NSMutableDictionary *d = [NSMutableDictionary dictionary];
  d[@"ApplicationType"] = type; d[@"CFBundleIdentifier"] = ident; d[@"CodeInfoIdentifier"] = ident; d[@"CompatibilityState"] = @0;
  d[@"IsContainerized"] = @NO; d[@"Path"] = path; d[@"SignerIdentity"] = @"Apple iPhone OS Application Signing";
  d[@"SignerOrganization"] = @"Apple Inc."; d[@"IsAdHocSigned"] = @YES; d[@"LSInstallType"] = @1;
  return d;
}
// identifiers of the extensions LaunchServices has under this app (empty when it does not know the app or lsd is not up);
// one in HOMED (given a data container here) counts only once LS has that container
static NSSet *registered(NSString *ident, NSSet *homed) {
  id app = ((id(*)(id,SEL,id))objc_msgSend)((id)NSClassFromString(@"LSApplicationProxy"), sel_registerName("applicationProxyForIdentifier:"), ident);
  NSArray *plugins = [app respondsToSelector:sel_registerName("plugInKitPlugins")] ? ((id(*)(id,SEL))objc_msgSend)(app, sel_registerName("plugInKitPlugins")) : nil;
  NSMutableSet *s = [NSMutableSet set];
  for (id p in plugins) { NSString *pid = ((id(*)(id,SEL))objc_msgSend)(p, sel_registerName("pluginIdentifier")); 
    if (pid && (![homed containsObject:pid] || ((id(*)(id,SEL))objc_msgSend)(p, sel_registerName("dataContainerURL")))) [s addObject:pid]; }
  return s;
}
int main(int argc, char **argv) {
  int failed = 0;
  @autoreleasepool {
    id ws = ((id(*)(id,SEL))objc_msgSend)((id)NSClassFromString(@"LSApplicationWorkspace"), sel_registerName("defaultWorkspace"));
    for (int i = 1; i < argc; i++) {
      NSString *app = [NSString stringWithUTF8String:argv[i]];
      NSDictionary *info = [NSDictionary dictionaryWithContentsOfFile:[app stringByAppendingPathComponent:@"Info.plist"]];
      NSString *ident = info[@"CFBundleIdentifier"];
      if (!ident) { printf("%s: no Info.plist identifier\n", argv[i]); continue; }
      NSMutableDictionary *d = base(ident, app, @"System");
      NSMutableDictionary *plugins = [NSMutableDictionary dictionary];
      NSMutableSet *homed = [NSMutableSet set];
      NSString *pdir = [app stringByAppendingPathComponent:@"PlugIns"];
      for (NSString *n in [[NSFileManager defaultManager] contentsOfDirectoryAtPath:pdir error:nil]) {
        if (![n.pathExtension isEqualToString:@"appex"]) continue;
        NSString *pp = [pdir stringByAppendingPathComponent:n];
        NSDictionary *pi = [NSDictionary dictionaryWithContentsOfFile:[pp stringByAppendingPathComponent:@"Info.plist"]];
        NSString *pid = pi[@"CFBundleIdentifier"];
        if (!pid) continue;
        NSMutableDictionary *p = base(pid, pp, @"PluginKitPlugin");
        p[@"PluginOwnerBundleID"] = ident;
        NSString *home = sandboxed(pp) ? container(pid) : nil;
        if (home) {
          p[@"IsContainerized"] = @YES; p[@"Container"] = home; [homed addObject:pid];
          p[@"EnvironmentVariables"] = @{ @"CFFIXED_USER_HOME": home, @"HOME": home, @"TMPDIR": [home stringByAppendingPathComponent:@"tmp"] };
        }
        plugins[pid] = p;
      }
      if (!plugins.count) continue;      // registering an app without extensions this way would only rewrite its record
      NSSet *want = [NSSet setWithArray:plugins.allKeys];
      if ([want isSubsetOfSet:registered(ident, homed)]) { printf("%s (%s): %lu extensions already registered\n", argv[i], ident.UTF8String, (unsigned long)want.count); continue; }
      d[@"_LSBundlePlugins"] = plugins;
      BOOL ok = ((BOOL(*)(id,SEL,id))objc_msgSend)(ws, sel_registerName("registerApplicationDictionary:"), d);
      NSMutableSet *now = [want mutableCopy];
      [now intersectSet:registered(ident, homed)];
      printf("%s (%s): %lu extensions, registerApplicationDictionary -> %d, %lu registered\n", argv[i], ident.UTF8String,
             (unsigned long)want.count, ok, (unsigned long)now.count);
      if (!ok || !now.count) failed++;   // a partial set is kept: some extensions may never register, retrying would not help
    }
  }
  return failed ? 1 : 0;
}
