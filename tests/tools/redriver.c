/* redriver.c - run the regexp parity table through the C regexp.c/regsub.c
 * (IMPLEMENTATION.md M5).
 *
 * Build against the objects of the C reference build:
 *   cc -O2 -fsigned-char -o target/c-ref/redriver tests/tools/redriver.c \
 *      target/c-ref/src/regexp.o target/c-ref/src/regsub.o
 * Run:
 *   target/c-ref/redriver < tests/fixtures/regexp/cases.txt \
 *      > tests/golden/<os>/regexp/report.txt
 *
 * The table format is described in tests/tools/retable.py.  For each case
 * the report has one line: the case number, then either "err <message>"
 * or "ok minlen first bol", the regexec result, the offsets of every set
 * start/end pair, and the regsub output.  regexp::tests::c_driver_table
 * produces the same report from the Rust port.
 *
 * Cases run in groups (a `+` flag continues the group), each group in a
 * fresh child process, because several inputs make C write outside its
 * buffers (a `[...]` class with bytes 128 to 255, Appendix B14, and some
 * error paths).  If a child dies, its current case is reported as
 * "crash <signal>" and the rest of its group as "skipped".
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

typedef struct regexp {
	char	*startp[10];
	char	*endp[10];
	int	minlen;
	char	first;
	char	bol;
	char	program[1];
} regexp;

extern regexp *regcomp(char *);
extern int regexec(regexp *re, char *str, int bol);
extern void regsub(regexp *re, char *src, char *dst);

char o_magic[1] = { 1 };
char o_ignorecase[1] = { 0 };

static char errmsg[256];

void regerror(char *txt)
{
	strncpy(errmsg, txt, sizeof errmsg - 1);
}

/* decode %HH and %% in place; returns the length */
static int decode(char *s)
{
	char *d = s, *p = s;
	while (*p) {
		if (p[0] == '%' && p[1] == '%') {
			*d++ = '%';
			p += 2;
		} else if (p[0] == '%' && p[1] && p[2]) {
			char hex[3] = { p[1], p[2], 0 };
			*d++ = (char)strtol(hex, NULL, 16);
			p += 3;
		} else {
			*d++ = *p++;
		}
	}
	*d = 0;
	return (int)(d - s);
}

static void put(const char *s)
{
	for (; *s; s++) {
		unsigned char c = (unsigned char)*s;
		if (c >= 0x20 && c < 0x7f && c != '%')
			putchar(c);
		else
			printf("%%%02X", c);
	}
}

static char out[65536], subj[4096];

/* one case; prints its report line when `print` is set */
static void run_case(int n, char **f, int print)
{
	regexp *re;
	char pat[4096], repl[4096];

	strcpy(pat, f[1]);
	strcpy(subj, f[2]);
	strcpy(repl, f[3]);
	decode(pat);
	decode(subj);
	o_magic[0] = strchr(f[0], 'm') != NULL;
	o_ignorecase[0] = strchr(f[0], 'i') != NULL;

	errmsg[0] = 0;
	re = regcomp(pat);
	if (print)
		printf("%d ", n);
	if (!re) {
		if (print) {
			printf("err ");
			put(errmsg);
			putchar('\n');
		}
		return;
	}
	if (print)
		/* bol is only tested for truth; B14 can leave any byte in it */
		printf("ok %d %d %d", re->minlen, re->first, re->bol != 0);
	if (regexec(re, subj, strchr(f[0], 'b') != NULL)) {
		int i;
		if (print)
			printf(" match");
		for (i = 0; i < 10; i++)
			if (print && re->startp[i] && re->endp[i])
				printf(" %d:%d-%d", i, (int)(re->startp[i] - subj),
				       (int)(re->endp[i] - subj));
		if (strcmp(repl, "-") != 0) {
			decode(repl);
			out[0] = 0;
			regsub(re, repl, out);
			if (print) {
				printf(" sub=");
				put(out);
			}
		}
	} else if (print) {
		printf(" nomatch");
	}
	if (print)
		putchar('\n');
	/* not freed: after B14 cases the heap may not survive free() */
}

#define MAXCASES 2000

int main(void)
{
	static char *lines[MAXCASES];
	static char buf[8192];
	int ncases = 0, first;

	while (fgets(buf, sizeof buf, stdin)) {
		if (ncases == MAXCASES)
			return 2;
		buf[strcspn(buf, "\n")] = 0;
		lines[ncases++] = strdup(buf);
	}

	for (first = 0; first < ncases;) {
		int end = first + 1, done = 0, fd[2], st, c;
		FILE *in;
		pid_t pid;

		while (end < ncases && lines[end][0] == '+')
			end++;
		fflush(stdout);
		if (pipe(fd) < 0)
			return 2;
		pid = fork();
		if (pid == 0) {
			int n;
			close(fd[0]);
			dup2(fd[1], 1);
			for (n = first; n < end; n++) {
				char *f[4], *p = lines[n];
				int k = 0;
				f[k++] = p;
				while (k < 4 && (p = strchr(p, '\t')) != NULL) {
					*p++ = 0;
					f[k++] = p;
				}
				if (k != 4)
					_exit(2);
						run_case(n + 1, f, 1);
				fflush(stdout);
			}
			_exit(0);
		}
		close(fd[1]);
		/* copy complete lines only; count them */
		in = fdopen(fd[0], "r");
		{
			static char rl[65536 * 4 + 64];
			while (fgets(rl, sizeof rl, in)) {
				if (strchr(rl, '\n')) {
					fputs(rl, stdout);
					done++;
				}
			}
		}
		fclose(in);
		waitpid(pid, &st, 0);
		if (!WIFEXITED(st) || WEXITSTATUS(st) != 0) {
			if (first + done < end)
				printf("%d crash %d\n", first + done + 1,
				       WIFSIGNALED(st) ? WTERMSIG(st) : 256 + WEXITSTATUS(st));
			for (c = first + done + 1; c < end; c++)
				printf("%d skipped\n", c + 1);
		}
		first = end;
	}
	return 0;
}
