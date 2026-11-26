// lsa BUNDLEID...: an app's LaunchServices record and the extensions registered under it
#import <Foundation/Foundation.h>
#import <objc/message.h>
int main(int argc, char **argv) {
  @autoreleasepool {
    Class P = NSClassFromString(@"LSApplicationProxy");
    for (int i = 1; i < argc; i++) {
      id app = ((id(*)(id,SEL,id))objc_msgSend)((id)P, sel_registerName("applicationProxyForIdentifier:"), [NSString stringWithUTF8String:argv[i]]);
      NSURL *url = [app respondsToSelector:sel_registerName("bundleURL")] ? ((id(*)(id,SEL))objc_msgSend)(app, sel_registerName("bundleURL")) : nil;
      NSArray *plugins = [app respondsToSelector:sel_registerName("plugInKitPlugins")] ? ((id(*)(id,SEL))objc_msgSend)(app, sel_registerName("plugInKitPlugins")) : nil;
      BOOL installed = [app respondsToSelector:sel_registerName("isInstalled")] ? ((BOOL(*)(id,SEL))objc_msgSend)(app, sel_registerName("isInstalled")) : NO;
      printf("%s: installed=%d url=%s plugins=%lu\n", argv[i], installed, url.path.UTF8String ?: "-", (unsigned long)plugins.count);
      for (id p in plugins) printf("   %s\n", [((id(*)(id,SEL))objc_msgSend)(p, sel_registerName("pluginIdentifier")) UTF8String]);
    }
  }
  return 0;
}
