/* capprobe.c - report what the host termcap emulation returns for every
 * capability elvis requests in starttcap(), plus tgoto() and tputs() output.
 *
 * Usage: capprobe OSPEED PCCHAR
 *   TERM / TERMINFO select the entry.  OSPEED is stored in the probe's own
 *   `ospeed` global and PCCHAR (decimal) in its `PC` global, mirroring the
 *   globals elvis defines in curses.c.  If the report changes with these
 *   values, the terminal library sees the program's globals (interposition).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern char *tgoto(char *cm, int destcol, int destrow);
extern char *tgetstr(char *id, char **bp);
extern int tputs(const char *cp, int affcnt, int (*outfn)(int));
extern int tgetent(char *bp, const char *name);
extern int tgetflag(char *id);
extern int tgetnum(char *id);

short ospeed;
char PC;

static char out[8192];
static int outlen;

static int collect(int ch)
{
	if (outlen < (int)sizeof out)
		out[outlen++] = (char)ch;
	return ch;
}

static void esc(const char *s, int n)
{
	int i;
	for (i = 0; i < n; i++) {
		unsigned char c = (unsigned char)s[i];
		if (c >= 0x20 && c < 0x7f && c != '\\')
			putchar(c);
		else
			printf("\\x%02x", c);
	}
}

static const char *strs[] = {
	"up", "vb", "cm", "so", "se", "ti", "te", "us", "ue", "md", "me",
	"as", "ae", "al", "dl", "ce", "cd", "sr", "im", "ei", "ic", "dc",
	"ks", "ke", "ku", "kd", "kl", "kr", "kP", "kN", "kh", "kH", 0
};
static const char *flags[] = { "am", "pt", "xo", "nx", 0 };
static const char *nums[] = { "ug", "li", "co", "pb", 0 };

int main(int argc, char **argv)
{
	char tbuf[4096];
	char area[4096];
	char *ap = area;
	char *cm;
	int i, r, c;

	if (argc != 3) {
		fprintf(stderr, "usage: capprobe OSPEED PCCHAR\n");
		return 64;
	}
	r = tgetent(tbuf, getenv("TERM"));
	printf("tgetent=%d\n", r);
	if (r != 1)
		return 0;

	ospeed = (short)atoi(argv[1]);
	PC = (char)atoi(argv[2]);
	printf("ospeed=%d PC=%d\n", ospeed, PC);

	for (i = 0; strs[i]; i++) {
		char *s = tgetstr((char *)strs[i], &ap);
		printf("str %s=", strs[i]);
		if (!s)
			printf("(null)");
		else {
			putchar('"');
			esc(s, (int)strlen(s));
			putchar('"');
		}
		putchar('\n');
	}
	for (i = 0; flags[i]; i++)
		printf("flag %s=%d\n", flags[i], tgetflag((char *)flags[i]));
	for (i = 0; nums[i]; i++)
		printf("num %s=%d\n", nums[i], tgetnum((char *)nums[i]));

	cm = tgetstr("cm", &ap);
	if (cm) {
		static const int rows[] = { 0, 1, 9, 10, 23, 99 };
		static const int cols[] = { 0, 1, 7, 79, 132 };
		for (r = 0; r < 6; r++)
			for (c = 0; c < 5; c++) {
				outlen = 0;
				tputs(tgoto(cm, cols[c], rows[r]), 1, collect);
				printf("cm row=%d col=%d \"", rows[r], cols[c]);
				esc(out, outlen);
				printf("\"\n");
			}
	}

	for (i = 0; strs[i]; i++) {
		char *s = tgetstr((char *)strs[i], &ap);
		int aff[2] = { 1, 24 };
		int k;
		if (!s)
			continue;
		for (k = 0; k < 2; k++) {
			outlen = 0;
			tputs(s, aff[k], collect);
			printf("tputs %s aff=%d \"", strs[i], aff[k]);
			esc(out, outlen);
			printf("\"\n");
		}
	}
	return 0;
}
