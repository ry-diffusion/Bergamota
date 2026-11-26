// axrt [0|1]: read or set Reduce Transparency (Accessibility > Display & Text Size) as user mobile, through libAccessibility.
// Build: xcrun --sdk iphoneos clang -arch arm64e -miphoneos-version-min=18.0 -O1 -o axrt axrt.c
//        codesign -f -s - axrt && python3 ../renegade/pacstrip.py --sign axrt
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(int c, char **v) {
	setenv("HOME", "/var/mobile", 1);
	if (setgid(501) || setuid(501)) { perror("setuid mobile"); return 1; }
	void *h = dlopen("/usr/lib/libAccessibility.dylib", RTLD_NOW);
	unsigned char (*get)(void) = h ? dlsym(h, "_AXSEnhanceBackgroundContrastEnabled") : NULL;
	void (*set)(unsigned char) = h ? dlsym(h, "_AXSSetEnhanceBackgroundContrastEnabled") : NULL;
	if (!get || !set) { fprintf(stderr, "axrt: libAccessibility: %s\n", dlerror()); return 1; }
	printf("reduce transparency was %d\n", get());
	if (c > 1) { set(atoi(v[1]) != 0); printf("now %d\n", get()); }
	return 0;
}
