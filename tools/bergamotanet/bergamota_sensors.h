/*
 * /dev/bergamota-sensors: the phone's motion and environment sensors, sample by sample, from the kernel (a driver, or
 * root's test writes) to the code in backboardd that turns them into the HID events iOS's own consumers read
 * (tools/bergamotanet/sensorbridge.c).
 *
 * Ported from Jesvi iOS / HakiOS (S20 port), bsd/sys/redwood_sensors.h, Apple Public Source License 2.0 (see
 * APPLE_LICENSE in this tree). Same sample layout and semantics; "redwood" became "bergamota". The ssp/tcs3407 hooks and
 * the proximity-emitter ioctl are not carried over (the Poco F1 has no sensor driver yet).
 *
 * A queue: read() blocks until there is at least one sample and returns as many whole ones as fit. The queue only fills
 * while a reader has the device open; a reader 256 samples behind loses the oldest. One shared queue: each sample goes
 * to one read() call, so run one reader (the bridge).
 *
 * Values are in Android's frame (x right, y up, z out of the screen):
 *   ACCEL      v[0..2] x, y, z; scale = the range in g (32768 counts are scale g). Face up z is +1 g (iOS: -1 g)
 *   GYRO       v[0..2] x, y, z in counts (the bridge's gyro_k turns them into degrees/s)
 *   MAG        v[0..2] x, y, z in counts (mag_k: uT), v[3] the accuracy 0-3
 *   PRESSURE   v[0] the pressure, v[1] the temperature (no HID consumer)
 *   LIGHT      v[0] lux, v[1] colour temperature in K (0: none), v[2] BERGAMOTA_LIGHT_FRONT or _REAR, v[3] raw clear
 *   PROXIMITY  v[0] 1 when something is near, 0 when not; v[1] the raw reading
 *
 * Boot-arg bergamotasensors (bit mask): 1 = make the device; 2 = root may write() whole samples into it (test path,
 * see bsd/dev/bergamota_sensors.c). Default 0: no device.
 */
#ifndef _SYS_BERGAMOTA_SENSORS_H_
#define _SYS_BERGAMOTA_SENSORS_H_

#include <stdint.h>

#define BERGAMOTA_SENSORS_NAME    "bergamota-sensors"
#define BERGAMOTA_SENSORS_DEVICE  "/dev/bergamota-sensors"

#define BERGAMOTA_SENSOR_ACCEL     1
#define BERGAMOTA_SENSOR_GYRO      2
#define BERGAMOTA_SENSOR_MAG       3
#define BERGAMOTA_SENSOR_PRESSURE  4
#define BERGAMOTA_SENSOR_LIGHT     5
#define BERGAMOTA_SENSOR_PROXIMITY 6

#define BERGAMOTA_LIGHT_FRONT      0
#define BERGAMOTA_LIGHT_REAR       1

struct bergamota_sensor_sample {
	uint64_t time;                  /* mach_absolute_time() when the kernel got it (0 in a test write: filled in) */
	uint64_t hub_ns;                /* the sensor hub's own time stamp in ns (0 if none) */
	uint32_t type;                  /* BERGAMOTA_SENSOR_* */
	int32_t scale;                  /* ACCEL: the range in g; else 0 */
	int32_t v[4];
};

#ifdef KERNEL
/* bsd/dev/bergamota_sensors.c: queue a sample for the readers (thread context only: it takes a mutex) */
void bergamota_sensors_post(const struct bergamota_sensor_sample *s);
int bergamota_sensors_readers(void);
void bergamota_sensors_init(void);      /* called from bergamota_ufs.c with the other drivers */
#endif
#endif /* _SYS_BERGAMOTA_SENSORS_H_ */
