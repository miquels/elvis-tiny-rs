/* maxrss PROG ARGS...: run PROG (stdin inherited), print exit status and its ru_maxrss in KiB. */
#include <spawn.h>
#include <stdio.h>
#include <sys/resource.h>
#include <sys/wait.h>
extern char **environ;
int main(int argc, char **argv) {
	pid_t pid; int st; struct rusage ru;
	if (argc < 2 || posix_spawn(&pid, argv[1], NULL, NULL, argv + 1, environ)) return 127;
	wait4(pid, &st, 0, &ru);
	fprintf(stderr, "status=%d signal=%d maxrss=%ld\n", WIFEXITED(st) ? WEXITSTATUS(st) : -1, WIFSIGNALED(st) ? WTERMSIG(st) : 0, ru.ru_maxrss);
	return 0;
}
