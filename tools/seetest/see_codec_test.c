/* Host test of the protobuf/float helpers of the SLPI SEE client (patch 1151): the code between the two markers of
 * beryllium_wlan.c is pasted here by tools/seetest/run.sh, then compared with known-good encodings (docs/SENSORS_SLPI.md 4.3). */
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#define INT32_MAXX INT32_MAX
#include "see_core.inc"

static int bad;
static void
eq(const char *what, const uint8_t *g, unsigned gn, const char *hex)
{
	char b[512] = "";
	for (unsigned i = 0; i < gn; i++) {
		sprintf(b + strlen(b), "%s%02x", i ? " " : "", g[i]);
	}
	if (strcmp(b, hex)) {
		printf("FAIL %s\n got  %s\nwant %s\n", what, b, hex);
		bad++;
	} else {
		printf("ok %s\n", what);
	}
}

int
main(void)
{
	uint8_t pb[128], body[48];
	uint32_t n = 0;
	body[n++] = 0x0a; body[n++] = 5; memcpy(body + n, "accel", 5); n += 5;
	body[n++] = 0x10; body[n++] = 0; body[n++] = 0x18; body[n++] = 1;
	uint32_t l = pb_creq(pb, 0xababababababababull, 0xababababababababull, 512, body, n);
	eq("SUID accel", pb, l, "0a 12 09 ab ab ab ab ab ab ab ab 11 ab ab ab ab ab ab ab ab 15 00 02 00 00 1a 04 08 01 10 00 22 0d 12 0b 0a 05 61 63 63 65 6c 10 00 18 01");
	uint32_t f50 = see_f_from_uint(50);
	uint8_t eb[8] = { 0x0d };
	memcpy(eb + 1, &f50, 4);
	l = pb_creq(pb, 0x1122334455667788ull, 0x99aabbccddeeff00ull, 513, eb, 5);
	eq("enable 50 Hz", pb, l, "0a 12 09 88 77 66 55 44 33 22 11 11 00 ff ee dd cc bb aa 99 15 01 02 00 00 1a 04 08 01 10 00 22 07 12 05 0d 00 00 48 42");
	l = pb_creq(pb, 0x1122334455667788ull, 0x99aabbccddeeff00ull, 10, NULL, 0);
	eq("disable", pb, l, "0a 12 09 88 77 66 55 44 33 22 11 11 00 ff ee dd cc bb aa 99 15 0a 00 00 00 1a 04 08 01 10 00 22 00");
	struct { float f; int64_t micro; } t[] = { { 9.81f, 9810000 }, { -0.2f, -200000 }, { 50.0f, 50000000 }, { 0.001f, 1000 }, { 1234.5f, 1234500000 } };
	for (unsigned i = 0; i < sizeof(t) / sizeof(t[0]); i++) {
		uint32_t bits;
		memcpy(&bits, &t[i].f, 4);
		int64_t m = see_f2micro(bits);
		int64_t d = m - t[i].micro;
		if (d < 0) d = -d;
		printf("%s f2micro(%g) = %lld\n", d <= 1000 ? "ok" : "FAIL", t[i].f, (long long)m);
		bad += d > 1000;
	}
	for (uint32_t k = 1; k <= 400; k += 37) {
		float f = (float)k;
		uint32_t want;
		memcpy(&want, &f, 4);
		if (see_f_from_uint(k) != want) {
			printf("FAIL see_f_from_uint(%u)\n", k);
			bad++;
		}
	}
	/* a decoded accel sample: x 0.1, y 9.81, z -0.2 m/s^2 -> counts of a 16 g range */
	float v[3] = { 0.1f, 9.81f, -0.2f };
	for (unsigned i = 0; i < 3; i++) {
		uint32_t b;
		memcpy(&b, &v[i], 4);
		printf("accel counts %d\n", see_clamp32(see_f2micro(b) * 2048 / 9806650));
	}
	printf("%s\n", bad ? "FAILED" : "all ok");
	return bad;
}
