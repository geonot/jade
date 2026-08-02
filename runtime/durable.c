#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>
#include "jinn_rt.h"
int jinn_fsync_checked(int fd, const char *what) {
    if (fd < 0) return -1;
    if (fsync(fd) != 0) {
        fprintf(stderr, "jinn: fsync failed for %s: %s — data may not be durable\n",
                what ? what : "(file)", strerror(errno));
        return -1;
    }
    return 0;
}
int jinn_dir_fsync(const char *filepath) {
    if (!filepath) return -1;
    char dirbuf[4096];
    const char *slash = strrchr(filepath, '/');
    const char *dir;
    if (slash && (size_t)(slash - filepath) < sizeof(dirbuf)) {
        size_t n = (size_t)(slash - filepath);
        if (n == 0) n = 1;
        memcpy(dirbuf, filepath, n);
        dirbuf[n] = '\0';
        dir = dirbuf;
    } else {
        dir = ".";
    }
    int dfd = open(dir, O_RDONLY | O_DIRECTORY);
    if (dfd < 0) {
        fprintf(stderr, "jinn: cannot open directory of %s for fsync: %s\n",
                filepath, strerror(errno));
        return -1;
    }
    int rc = jinn_fsync_checked(dfd, dir);
    close(dfd);
    return rc;
}
int jinn_atomic_rewrite(const char *path, jinn_fill_fn fill, void *arg) {
    if (!path || !fill) return -1;
    size_t plen = strlen(path);
    char *tmp_path = (char *)malloc(plen + 12);
    if (!tmp_path) return -1;
    memcpy(tmp_path, path, plen);
    memcpy(tmp_path + plen, ".tmpXXXXXX", 11);
    int tfd = mkstemp(tmp_path);
    if (tfd < 0) {
        fprintf(stderr, "jinn: cannot create temp file for %s: %s\n",
                path, strerror(errno));
        free(tmp_path);
        return -1;
    }
    FILE *tmp = fdopen(tfd, "w+b");
    if (!tmp) {
        close(tfd);
        unlink(tmp_path);
        free(tmp_path);
        return -1;
    }
    int rc = fill(tmp, arg);
    if (rc == 0 && fflush(tmp) != 0) {
        fprintf(stderr, "jinn: write to temp for %s failed: %s\n",
                path, strerror(errno));
        rc = -1;
    }
    if (rc == 0) rc = jinn_fsync_checked(fileno(tmp), tmp_path);
    if (fclose(tmp) != 0 && rc == 0) {
        fprintf(stderr, "jinn: closing temp for %s failed: %s\n",
                path, strerror(errno));
        rc = -1;
    }
    if (rc == 0 && rename(tmp_path, path) != 0) {
        fprintf(stderr, "jinn: rename %s -> %s failed: %s\n",
                tmp_path, path, strerror(errno));
        rc = -1;
    }
    if (rc == 0) {
        rc = jinn_dir_fsync(path);
    } else {
        unlink(tmp_path);
    }
    free(tmp_path);
    return rc;
}
int jinn_atomic_rewrite_reopen(const char *path, jinn_fill_fn fill, void *arg,
                               FILE **fpp) {
    if (jinn_atomic_rewrite(path, fill, arg) != 0) return -1;
    if (fpp) {
        FILE *nf = fopen(path, "r+b");
        if (!nf) {
            fprintf(stderr, "jinn: reopen after rewrite of %s failed: %s\n",
                    path, strerror(errno));
            return -1;
        }
        fseek(nf, 0, SEEK_END);
        if (*fpp) fclose(*fpp);
        *fpp = nf;
    }
    return 0;
}
int jinn_writer_lock(const char *path) {
    if (!path) return -1;
    size_t plen = strlen(path);
    char *lock_path = (char *)malloc(plen + 6);
    if (!lock_path) return -1;
    memcpy(lock_path, path, plen);
    memcpy(lock_path + plen, ".lock", 6);
    int fd = open(lock_path, O_CREAT | O_RDWR | O_CLOEXEC, 0644);
    if (fd < 0) {
        fprintf(stderr, "jinn: cannot create lock file %s: %s\n",
                lock_path, strerror(errno));
        free(lock_path);
        return -1;
    }
    if (flock(fd, LOCK_EX | LOCK_NB) != 0) {
        fprintf(stderr,
                "jinn: store '%s' is already open for writing by another process "
                "(the store layer is single-writer; close the other process, or "
                "point this one at its own store)\n",
                path);
        close(fd);
        free(lock_path);
        return -1;
    }
    free(lock_path);
    return fd;
}
void jinn_writer_unlock(int lock_fd) {
    if (lock_fd >= 0) close(lock_fd);
}
