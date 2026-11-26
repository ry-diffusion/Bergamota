// lsplug /Applications/X.app...: register a system app with its embedded extensions (PlugIns/*.appex) in LaunchServices,
// the way uicache does (registerApplicationDictionary: with _LSBundlePlugins). For system apps whose extensions LS never took:
// on the iOS 26 root every /Applications app had 0 plugins, so chronod found no widget/control providers (home widget "Waiting...",
// Control Center layout regenerated empty, controls gallery would not open). registerApplication: / registerPlugin: did not help.
// Build: xcrun --sdk iphoneos clang -arch arm64e -miphoneos-version-min=18.0 -fobjc-arc -O1 -o lsplug lsplug.m -framework Foundation
//        codesign -f -s - --entitlements lsplug.ents lsplug && python3 ../renegade/pacstrip.py --sign lsplug
// Run on the phone (root): lsplug /Applications/*.app  (only apps with a PlugIns folder matter; ~80 take a few minutes)
#import <Foundation/Foundation.h>
#import <objc/message.h>
static NSMutableDictionary *base(NSString *ident, NSString *path, NSString *type) {
  NSMutableDictionary *d = [NSMutableDictionary dictionary];
  d[@"ApplicationType"] = type; d[@"CFBundleIdentifier"] = ident; d[@"CodeInfoIdentifier"] = ident; d[@"CompatibilityState"] = @0;
  d[@"IsContainerized"] = @NO; d[@"Path"] = path; d[@"SignerIdentity"] = @"Apple iPhone OS Application Signing";
  d[@"SignerOrganization"] = @"Apple Inc."; d[@"IsAdHocSigned"] = @YES; d[@"LSInstallType"] = @1;
  return d;
}
int main(int argc, char **argv) {
  @autoreleasepool {
    id ws = ((id(*)(id,SEL))objc_msgSend)((id)NSClassFromString(@"LSApplicationWorkspace"), sel_registerName("defaultWorkspace"));
    for (int i = 1; i < argc; i++) {
      NSString *app = [NSString stringWithUTF8String:argv[i]];
      NSDictionary *info = [NSDictionary dictionaryWithContentsOfFile:[app stringByAppendingPathComponent:@"Info.plist"]];
      NSString *ident = info[@"CFBundleIdentifier"];
      if (!ident) { printf("%s: no Info.plist identifier\n", argv[i]); continue; }
      NSMutableDictionary *d = base(ident, app, @"System");
      NSMutableDictionary *plugins = [NSMutableDictionary dictionary];
      NSString *pdir = [app stringByAppendingPathComponent:@"PlugIns"];
      for (NSString *n in [[NSFileManager defaultManager] contentsOfDirectoryAtPath:pdir error:nil]) {
        if (![n.pathExtension isEqualToString:@"appex"]) continue;
        NSString *pp = [pdir stringByAppendingPathComponent:n];
        NSDictionary *pi = [NSDictionary dictionaryWithContentsOfFile:[pp stringByAppendingPathComponent:@"Info.plist"]];
        NSString *pid = pi[@"CFBundleIdentifier"];
        if (!pid) continue;
        NSMutableDictionary *p = base(pid, pp, @"PluginKitPlugin");
        p[@"PluginOwnerBundleID"] = ident;
        plugins[pid] = p;
      }
      d[@"_LSBundlePlugins"] = plugins;
      BOOL ok = ((BOOL(*)(id,SEL,id))objc_msgSend)(ws, sel_registerName("registerApplicationDictionary:"), d);
      printf("%s (%s): %lu extensions, registerApplicationDictionary -> %d\n", argv[i], ident.UTF8String, (unsigned long)plugins.count, ok);
    }
  }
  return 0;
}
