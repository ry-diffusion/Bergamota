/*
 * fgjob OUTFILE CMD [ARGS]: run CMD as a transient launchd job with ProcessType Interactive (stdout and stderr appended to
 * OUTFILE), wait for it to exit, remove the job. Nothing persists: launchd forgets submitted jobs at reboot.
 * Why: dropbear's job (org.sweet.dropbear, no ProcessType) is darwin-background, and so is everything it starts: priority 4,
 * sleeps coalesced by up to 100 ms (kern.timer_coalesce_bg_ns_max). Neither QoS, role, task category, a spawn process type
 * nor clearing PRIO_DARWIN_PROCESS lifts it from inside the session. Under fgjob the same 8 ms sleep is ~1 ms late
 * (priority 31). Used for `sweetwake drag`, whose finger otherwise froze for 0.1-1.5 s.
 * Build: launch.h is only in the macOS SDK, e.g.
 *   mkdir -p /tmp/inc && cp "$(xcrun --sdk macosx --show-sdk-path)/usr/include/launch.h" /tmp/inc/
 *   xcrun -sdk iphoneos clang -arch arm64e -miphoneos-version-min=18.0 -O2 -Wno-deprecated-declarations -I/tmp/inc fgjob.c -o fgjob
 *   then codesign -s - and tools/renegade/pacstrip.py --sign.
 */
#include <launch.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
static launch_data_t str(const char *s) { return launch_data_new_string(s); }
int main(int c, char **v)
{
	if (c < 3) { fprintf(stderr, "usage: fgjob OUTFILE CMD [ARGS]\n"); return 2; }
	char label[64]; snprintf(label, sizeof(label), "org.sweet.fgjob.%d", getpid());
	launch_data_t job = launch_data_alloc(LAUNCH_DATA_DICTIONARY), args = launch_data_alloc(LAUNCH_DATA_ARRAY);
	for (int i = 2; i < c; i++) launch_data_array_set_index(args, str(v[i]), i - 2);
	launch_data_dict_insert(job, str(label), LAUNCH_JOBKEY_LABEL);
	launch_data_dict_insert(job, args, LAUNCH_JOBKEY_PROGRAMARGUMENTS);
	launch_data_dict_insert(job, str("Interactive"), "ProcessType");
	launch_data_dict_insert(job, str(v[1]), LAUNCH_JOBKEY_STANDARDOUTPATH);
	launch_data_dict_insert(job, str(v[1]), LAUNCH_JOBKEY_STANDARDERRORPATH);
	launch_data_dict_insert(job, launch_data_new_bool(1), LAUNCH_JOBKEY_RUNATLOAD);
	launch_data_dict_insert(job, launch_data_new_bool(0), LAUNCH_JOBKEY_KEEPALIVE);
	launch_data_t msg = launch_data_alloc(LAUNCH_DATA_DICTIONARY);
	launch_data_dict_insert(msg, job, LAUNCH_KEY_SUBMITJOB);
	launch_data_t r = launch_msg(msg);
	if (!r) { perror("fgjob: launch_msg"); return 1; }
	if (launch_data_get_type(r) == LAUNCH_DATA_ERRNO && launch_data_get_errno(r)) { fprintf(stderr, "fgjob: submit: errno %d\n", launch_data_get_errno(r)); return 1; }
	/* wait until the job has no PID any more */
	for (int i = 0; i < 6000; i++) {
		usleep(i < 10 ? 100000 : 50000);
		launch_data_t q = launch_data_alloc(LAUNCH_DATA_DICTIONARY);
		launch_data_dict_insert(q, str(label), LAUNCH_KEY_GETJOB);
		launch_data_t j = launch_msg(q);
		if (!j || launch_data_get_type(j) != LAUNCH_DATA_DICTIONARY) break;
		if (i >= 10 && !launch_data_dict_lookup(j, LAUNCH_JOBKEY_PID)) break;
	}
	launch_data_t rm = launch_data_alloc(LAUNCH_DATA_DICTIONARY);
	launch_data_dict_insert(rm, str(label), LAUNCH_KEY_REMOVEJOB);
	launch_msg(rm);
	return 0;
}
