/* padprobe.c - count the pad characters libtinfo's tputs emits for each ospeed
 * value (c-behavior.md, Linux "Padding").  Build: cc -o padprobe padprobe.c -ltinfo
 * Run on a pty with TERM and TERMINFO set (tests/tools/ptyrun.py).  */
#include <stdio.h>
#include <stdlib.h>
extern int tgetent(char *, const char *);
extern int tputs(const char *, int, int (*)(int));
short ospeed = -7; char PC = 'Z';
static int n, other;
static int cnt(int c) { if (c == PC) n++; else other++; return c; }
int main(int argc, char **argv) {
	char b[4096];
	int r = tgetent(b, getenv("TERM"));
	printf("tgetent=%d after: ospeed=%d PC=%d\n", r, ospeed, PC);
	static const short codes[] = {0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,4096,4097,4098,4099,4100,4101,4111,4112,-1,-32768};
	PC = 0;
	for (unsigned i = 0; i < sizeof codes / sizeof *codes; i++) {
		ospeed = codes[i];
		printf("ospeed=%d", ospeed);
		const char *caps[] = {"$<50>", "$<1000>", "$<10*>", "$<1000/>", "$<.5>", "$<3.7*>", 0};
		for (int k = 0; caps[k]; k++) { n = other = 0; tputs(caps[k], 24, cnt); printf(" %s:%d%s", caps[k], n, other ? "+other" : ""); }
		putchar('\n');
	}
	return 0;
}
