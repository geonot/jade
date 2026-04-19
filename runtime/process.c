/* runtime/process.c — Subprocess spawning helpers */
#include <unistd.h>
#include <sys/wait.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <spawn.h>
#include <errno.h>

extern char **environ;

/* Run a command string via /bin/sh -c and capture stdout into buf.
 * Returns the number of bytes read, or -1 on error.
 * exit_code is set to the child's exit status. */
long jade_popen_read(const char *cmd, char *buf, long buf_size, int *exit_code) {
    FILE *fp = popen(cmd, "r");
    if (!fp) { *exit_code = -1; return -1; }
    long total = 0;
    while (total < buf_size - 1) {
        size_t n = fread(buf + total, 1, (size_t)(buf_size - 1 - total), fp);
        if (n == 0) break;
        total += (long)n;
    }
    buf[total] = '\0';
    int status = pclose(fp);
    *exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
    return total;
}

/* Run a command via system() and return the exit code. */
int jade_system(const char *cmd) {
    int status = system(cmd);
    if (status == -1) return -1;
    return WIFEXITED(status) ? WEXITSTATUS(status) : -1;
}

/* Fork+exec with pipe for stdout capture.
 * argv is a NULL-terminated array of strings.
 * Returns bytes read into buf; exit_code set to child status. */
long jade_exec_capture(const char *prog, char *const argv[],
                       char *buf, long buf_size, int *exit_code) {
    int pipefd[2];
    if (pipe(pipefd) < 0) { *exit_code = -1; return -1; }

    pid_t pid = fork();
    if (pid < 0) { *exit_code = -1; return -1; }

    if (pid == 0) {
        /* child */
        close(pipefd[0]);
        dup2(pipefd[1], STDOUT_FILENO);
        dup2(pipefd[1], STDERR_FILENO);
        close(pipefd[1]);
        execvp(prog, argv);
        _exit(127);
    }

    /* parent */
    close(pipefd[1]);
    long total = 0;
    while (total < buf_size - 1) {
        ssize_t n = read(pipefd[0], buf + total, (size_t)(buf_size - 1 - total));
        if (n <= 0) break;
        total += (long)n;
    }
    buf[total] = '\0';
    close(pipefd[0]);

    int status;
    waitpid(pid, &status, 0);
    *exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
    return total;
}
