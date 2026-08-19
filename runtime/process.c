#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif
#include <unistd.h>
#include <sys/wait.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <spawn.h>
#include <errno.h>
#include <signal.h>
#include <time.h>
#include <stdint.h>
#include "jinn_rt.h"

extern char **environ;

typedef struct { void *ptr; int64_t len; int64_t cap; } jinn_vec_hdr_t;
static const char *sso_data_proc(const jinn_sso_t *s, int64_t *out_len) {
    if ((unsigned char)s->bytes[23] & 0x80u) {
        const char *p; int64_t l;
        memcpy(&p, s->bytes,     8);
        memcpy(&l, s->bytes + 8, 8);
        *out_len = l; return p;
    }
    *out_len = 23 - (int64_t)(unsigned char)s->bytes[23];
    return s->bytes;
}
static char **jinn_vec_to_argv(const jinn_vec_hdr_t *vec) {
    if (!vec || vec->len < 1) { errno = EINVAL; return NULL; }
    int64_t n = vec->len;
    const jinn_sso_t *elems = (const jinn_sso_t *)vec->ptr;
    char **argv = (char **)malloc((size_t)(n + 1) * sizeof(char *));
    if (!argv) return NULL;
    for (int64_t i = 0; i < n; i++) {
        int64_t len;
        const char *data = sso_data_proc(&elems[i], &len);
        char *cstr = (char *)malloc((size_t)(len + 1));
        if (!cstr) {
            for (int64_t j = 0; j < i; j++) free(argv[j]);
            free(argv); return NULL;
        }
        memcpy(cstr, data, (size_t)len);
        cstr[len] = '\0';
        argv[i] = cstr;
    }
    argv[n] = NULL;
    return argv;
}
static void free_argv_proc(char **argv, int64_t n) {
    if (!argv) return;
    for (int64_t i = 0; i < n; i++) free(argv[i]);
    free(argv);
}
static int shell_enabled(void) {
    const char *v = getenv("JINN_ALLOW_SHELL");
    return v && strcmp(v, "1") == 0;
}
static int wait_child_with_timeout(pid_t pid, int *exit_code, long timeout_ms) {
    if (timeout_ms <= 0) {
        int status;
        if (waitpid(pid, &status, 0) < 0) {
            *exit_code = -1;
            return -1;
        }
        *exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
        return 0;
    }
    struct timespec start;
    if (clock_gettime(CLOCK_MONOTONIC, &start) != 0) {
        *exit_code = -1;
        return -1;
    }
    for (;;) {
        int status;
        pid_t r = waitpid(pid, &status, WNOHANG);
        if (r == pid) {
            *exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
            return 0;
        }
        if (r < 0) {
            *exit_code = -1;
            return -1;
        }
        struct timespec now;
        if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
            kill(pid, SIGKILL);
            *exit_code = -1;
            return -1;
        }
        long elapsed_ms = (now.tv_sec - start.tv_sec) * 1000L
            + (now.tv_nsec - start.tv_nsec) / 1000000L;
        if (elapsed_ms >= timeout_ms) {
            kill(pid, SIGKILL);
            waitpid(pid, NULL, 0);
            *exit_code = -1;
            errno = ETIMEDOUT;
            return -1;
        }
        struct timespec ns = {0, 1000000};
        nanosleep(&ns, NULL);
    }
}
static long proc_read_all_fd(int fd, char **out_buf) {
    long cap = 65536, total = 0;
    char *buf = (char *)malloc((size_t)cap);
    if (!buf) return -1;
    for (;;) {
        if (total == cap - 1) {
            long ncap = cap * 2;
            char *nb = (char *)realloc(buf, (size_t)ncap);
            if (!nb) {
                free(buf);
                return -1;
            }
            buf = nb;
            cap = ncap;
        }
        ssize_t n = read(fd, buf + total, (size_t)(cap - 1 - total));
        if (n <= 0) break;
        total += (long)n;
    }
    buf[total] = '\0';
    char *shrunk = (char *)realloc(buf, (size_t)(total + 1));
    *out_buf = shrunk ? shrunk : buf;
    return total;
}
static long proc_read_all_fp(FILE *fp, char **out_buf) {
    long cap = 65536, total = 0;
    char *buf = (char *)malloc((size_t)cap);
    if (!buf) return -1;
    for (;;) {
        if (total == cap - 1) {
            long ncap = cap * 2;
            char *nb = (char *)realloc(buf, (size_t)ncap);
            if (!nb) {
                free(buf);
                return -1;
            }
            buf = nb;
            cap = ncap;
        }
        size_t n = fread(buf + total, 1, (size_t)(cap - 1 - total), fp);
        if (n == 0) break;
        total += (long)n;
    }
    buf[total] = '\0';
    char *shrunk = (char *)realloc(buf, (size_t)(total + 1));
    *out_buf = shrunk ? shrunk : buf;
    return total;
}
char *jinn_popen_read_all(const char *cmd, long *out_len, int *exit_code) {
    if (!out_len || !exit_code) {
        errno = EINVAL;
        return NULL;
    }
    *out_len = -1;
    if (!shell_enabled()) {
        *exit_code = -1;
        errno = EPERM;
        return NULL;
    }
    FILE *fp = popen(cmd, "r");
    if (!fp) {
        *exit_code = -1;
        return NULL;
    }
    char *buf = NULL;
    long total = proc_read_all_fp(fp, &buf);
    int status = pclose(fp);
    *exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
    if (total < 0) {
        *exit_code = -1;
        return NULL;
    }
    *out_len = total;
    return buf;
}
char *jinn_spawn_capture_all(const void *vec_ptr, long *out_len, int *exit_code) {
    if (!out_len || !exit_code) {
        errno = EINVAL;
        return NULL;
    }
    *out_len = -1;
    if (!vec_ptr) {
        *exit_code = -1;
        errno = EINVAL;
        return NULL;
    }
    const jinn_vec_hdr_t *vec = (const jinn_vec_hdr_t *)vec_ptr;
    char **argv = jinn_vec_to_argv(vec);
    if (!argv) {
        *exit_code = -1;
        return NULL;
    }
    int pipefd[2];
    if (pipe(pipefd) < 0) {
        free_argv_proc(argv, vec->len);
        *exit_code = -1;
        return NULL;
    }
    pid_t pid = fork();
    if (pid < 0) {
        close(pipefd[0]);
        close(pipefd[1]);
        free_argv_proc(argv, vec->len);
        *exit_code = -1;
        return NULL;
    }
    if (pid == 0) {
        close(pipefd[0]);
        dup2(pipefd[1], STDOUT_FILENO);
        dup2(pipefd[1], STDERR_FILENO);
        close(pipefd[1]);
        execvp(argv[0], (char *const *)argv);
        _exit(127);
    }
    close(pipefd[1]);
    char *buf = NULL;
    long total = proc_read_all_fd(pipefd[0], &buf);
    close(pipefd[0]);
    free_argv_proc(argv, vec->len);
    if (wait_child_with_timeout(pid, exit_code, 0) != 0 || total < 0) {
        free(buf);
        *exit_code = total < 0 ? -1 : *exit_code;
        return NULL;
    }
    *out_len = total;
    return buf;
}
long jinn_popen_read(const char *cmd, char *buf, long buf_size, int *exit_code) {
    if (!buf || buf_size <= 0 || !exit_code) {
        errno = EINVAL;
        if (exit_code) *exit_code = -1;
        return -1;
    }
    if (!shell_enabled()) {
        *exit_code = -1;
        errno = EPERM;
        return -1;
    }
    FILE *fp = popen(cmd, "r");
    if (!fp) {
        *exit_code = -1;
        return -1;
    }
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
int jinn_system(const char *cmd) {
    if (!shell_enabled()) {
        errno = EPERM;
        return -1;
    }
    int status = system(cmd);
    if (status == -1) return -1;
    return WIFEXITED(status) ? WEXITSTATUS(status) : -1;
}
int jinn_exec_argv_timeout(const char *prog, char *const argv[], int *exit_code, long timeout_ms) {
    if (!prog || !argv || !exit_code) {
        errno = EINVAL;
        if (exit_code) *exit_code = -1;
        return -1;
    }
    pid_t pid = fork();
    if (pid < 0) {
        *exit_code = -1;
        return -1;
    }
    if (pid == 0) {
        execvp(prog, argv);
        _exit(127);
    }
    return wait_child_with_timeout(pid, exit_code, timeout_ms);
}
int jinn_exec_argv(const char *prog, char *const argv[], int *exit_code) {
    return jinn_exec_argv_timeout(prog, argv, exit_code, 0);
}

long jinn_exec_argv_capture_timeout(const char *prog, char *const argv[],
                                    char *buf, long buf_size,
                                    int *exit_code, long timeout_ms) {
    if (!prog || !argv || !buf || buf_size <= 0 || !exit_code) {
        errno = EINVAL;
        if (exit_code) *exit_code = -1;
        return -1;
    }
    int pipefd[2];
    if (pipe(pipefd) < 0) {
        *exit_code = -1;
        return -1;
    }
    pid_t pid = fork();
    if (pid < 0) {
        close(pipefd[0]);
        close(pipefd[1]);
        *exit_code = -1;
        return -1;
    }
    if (pid == 0) {
        close(pipefd[0]);
        dup2(pipefd[1], STDOUT_FILENO);
        dup2(pipefd[1], STDERR_FILENO);
        close(pipefd[1]);
        execvp(prog, argv);
        _exit(127);
    }
    close(pipefd[1]);
    long total = 0;
    while (total < buf_size - 1) {
        ssize_t n = read(pipefd[0], buf + total, (size_t)(buf_size - 1 - total));
        if (n <= 0) break;
        total += (long)n;
    }
    buf[total] = '\0';
    close(pipefd[0]);
    if (wait_child_with_timeout(pid, exit_code, timeout_ms) != 0) {
        return -1;
    }
    return total;
}
long jinn_exec_argv_capture(const char *prog, char *const argv[],
                            char *buf, long buf_size, int *exit_code) {
    return jinn_exec_argv_capture_timeout(prog, argv, buf, buf_size, exit_code, 0);
}

long jinn_exec_capture(const char *prog, char *const argv[],
                       char *buf, long buf_size, int *exit_code) {
    return jinn_exec_argv_capture(prog, argv, buf, buf_size, exit_code);
}

long jinn_spawn_capture(const void *vec_ptr, char *buf, long buf_size, int *exit_code) {
    if (!vec_ptr || !buf || buf_size <= 0 || !exit_code) {
        errno = EINVAL;
        if (exit_code) *exit_code = -1;
        return -1;
    }
    const jinn_vec_hdr_t *vec = (const jinn_vec_hdr_t *)vec_ptr;
    char **argv = jinn_vec_to_argv(vec);
    if (!argv) { *exit_code = -1; return -1; }

    long result = jinn_exec_argv_capture_timeout(argv[0], (char *const *)argv,
                                                  buf, buf_size, exit_code, 0);
    free_argv_proc(argv, vec->len);
    return result;
}
int jinn_spawn_exec(const void *vec_ptr, int *exit_code) {
    if (!vec_ptr || !exit_code) {
        errno = EINVAL;
        if (exit_code) *exit_code = -1;
        return -1;
    }
    const jinn_vec_hdr_t *vec = (const jinn_vec_hdr_t *)vec_ptr;
    char **argv = jinn_vec_to_argv(vec);
    if (!argv) { *exit_code = -1; return -1; }
    int result = jinn_exec_argv_timeout(argv[0], (char *const *)argv, exit_code, 0);
    free_argv_proc(argv, vec->len);
    return result;
}
