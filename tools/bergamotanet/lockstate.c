/* lockstate: print SpringBoard's lock state (notify key com.apple.springboard.lockstate: 0 unlocked, 1 locked) */
#include <notify.h>
#include <stdio.h>
int main(void)
{
	int tok; uint64_t st = 99;
	if (notify_register_check("com.apple.springboard.lockstate", &tok) != NOTIFY_STATUS_OK) return 1;
	notify_get_state(tok, &st);
	printf("lockstate %llu\n", (unsigned long long)st);
	return 0;
}
