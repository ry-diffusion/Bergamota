/*
 * sensorpost: write synthetic samples into /dev/bergamota-sensors (kernel boot-arg bergamotasensors=3, as root) to test
 * the sensor bridge (sensorbridge.c in backboardd) before a real sensor driver exists. Android axes: lying face up the
 * accelerometer reads z = +1 g, upright in portrait y = +1 g.
 *
 *   sensorpost accel X Y Z [N]        N samples (default 1) of X Y Z in g, 20 ms apart
 *   sensorpost rotate portrait|left|right|upside|faceup|facedown
 *                                     a 0.8 s burst of gravity along that side (left = rotated counter-clockwise,
 *                                     home indicator on the right: UIDeviceOrientationLandscapeLeft)
 *   sensorpost prox 1|0               proximity near / far
 *   sensorpost lux N [SECONDS]        ambient light N lux, repeated at 4 Hz (default 6 s; auto-brightness needs a stream)
 *
 * MIT License; part of the sensor plumbing ported from Jesvi iOS / HakiOS (see docs/SENSORS.md).
 */
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "bergamota_sensors.h"

static int fd;

static void
post(int type, int scale, int32_t a, int32_t b, int32_t c, int32_t d)
{
	struct bergamota_sensor_sample s = { .type = (uint32_t)type, .scale = scale, .v = { a, b, c, d } };

	if (write(fd, &s, sizeof(s)) != (ssize_t)sizeof(s)) {
		perror("sensorpost: write (kernel needs bergamotasensors=3, and root)");
		exit(1);
	}
}

static void
accel(double x, double y, double z, int n)
{
	const int scale = 8;                    /* the range in g: 32768 counts are 8 g */
	const double k = 32768.0 / scale;

	for (int i = 0; i < n; i++) {
		post(BERGAMOTA_SENSOR_ACCEL, scale, (int32_t)(x * k), (int32_t)(y * k), (int32_t)(z * k), 0);
		usleep(20000);
	}
}

static int
usage(void)
{
	fputs("usage: sensorpost accel X Y Z [N] | rotate portrait|left|right|upside|faceup|facedown | prox 1|0 | lux N [SECONDS]\n",
	    stderr);
	return 2;
}

int
main(int argc, char **argv)
{
	if (argc < 3) {
		return usage();
	}
	fd = open(BERGAMOTA_SENSORS_DEVICE, O_WRONLY);
	if (fd < 0) {
		perror("sensorpost: " BERGAMOTA_SENSORS_DEVICE);
		return 1;
	}
	if (!strcmp(argv[1], "accel") && argc >= 5) {
		accel(atof(argv[2]), atof(argv[3]), atof(argv[4]), argc > 5 ? atoi(argv[5]) : 1);
	} else if (!strcmp(argv[1], "rotate")) {
		static const struct { const char *name; double x, y, z; } side[] = {
			{ "portrait", 0, 1, 0 }, { "upside", 0, -1, 0 }, { "left", 1, 0, 0 }, { "right", -1, 0, 0 },
			{ "faceup", 0, 0, 1 }, { "facedown", 0, 0, -1 },
		};
		unsigned i;

		for (i = 0; i < sizeof(side) / sizeof(side[0]) && strcmp(argv[2], side[i].name); i++) {
		}
		if (i == sizeof(side) / sizeof(side[0])) {
			return usage();
		}
		accel(side[i].x, side[i].y, side[i].z, 40);
	} else if (!strcmp(argv[1], "prox")) {
		post(BERGAMOTA_SENSOR_PROXIMITY, 0, atoi(argv[2]) ? 1 : 0, 0, 0, 0);
	} else if (!strcmp(argv[1], "lux")) {
		int n = (argc > 3 ? atoi(argv[3]) : 6) * 4;

		for (int i = 0; i < n; i++) {
			post(BERGAMOTA_SENSOR_LIGHT, 0, atoi(argv[2]), 0, BERGAMOTA_LIGHT_FRONT, 0);
			usleep(250000);
		}
	} else {
		return usage();
	}
	return 0;
}
