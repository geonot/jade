#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
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
            return -2;
        }
        fseek(nf, 0, SEEK_END);
        if (*fpp) fclose(*fpp);
        *fpp = nf;
    }
    return 0;
}
struct JinnRewrite {
    char *tmp_path;
    char *path;
    FILE *tmp;
};
JinnRewrite *jinn_rewrite_begin(const char *path) {
    if (!path) return NULL;
    JinnRewrite *rw = (JinnRewrite *)calloc(1, sizeof(JinnRewrite));
    if (!rw) return NULL;
    size_t plen = strlen(path);
    rw->path = (char *)malloc(plen + 1);
    rw->tmp_path = (char *)malloc(plen + 12);
    if (!rw->path || !rw->tmp_path) {
        free(rw->path);
        free(rw->tmp_path);
        free(rw);
        return NULL;
    }
    memcpy(rw->path, path, plen + 1);
    memcpy(rw->tmp_path, path, plen);
    memcpy(rw->tmp_path + plen, ".tmpXXXXXX", 11);
    int tfd = mkstemp(rw->tmp_path);
    if (tfd < 0) {
        fprintf(stderr, "jinn: cannot create temp file for %s: %s\n", path, strerror(errno));
        free(rw->path);
        free(rw->tmp_path);
        free(rw);
        return NULL;
    }
    rw->tmp = fdopen(tfd, "w+b");
    if (!rw->tmp) {
        close(tfd);
        unlink(rw->tmp_path);
        free(rw->path);
        free(rw->tmp_path);
        free(rw);
        return NULL;
    }
    return rw;
}
FILE *jinn_rewrite_file(JinnRewrite *rw) { return rw ? rw->tmp : NULL; }
static void rewrite_free(JinnRewrite *rw) {
    free(rw->path);
    free(rw->tmp_path);
    free(rw);
}
void jinn_rewrite_abort(JinnRewrite *rw) {
    if (!rw) return;
    if (rw->tmp) fclose(rw->tmp);
    unlink(rw->tmp_path);
    rewrite_free(rw);
}
FILE *jinn_rewrite_commit(JinnRewrite *rw, FILE *old_fp) {
    if (!rw) return NULL;
    int rc = 0;
    if (fflush(rw->tmp) != 0) {
        fprintf(stderr, "jinn: write to temp for %s failed: %s\n", rw->path, strerror(errno));
        rc = -1;
    }
    if (rc == 0) rc = jinn_fsync_checked(fileno(rw->tmp), rw->tmp_path);
    if (fclose(rw->tmp) != 0 && rc == 0) {
        fprintf(stderr, "jinn: closing temp for %s failed: %s\n", rw->path, strerror(errno));
        rc = -1;
    }
    rw->tmp = NULL;
    if (rc == 0 && rename(rw->tmp_path, rw->path) != 0) {
        fprintf(stderr, "jinn: rename %s -> %s failed: %s\n", rw->tmp_path, rw->path,
                strerror(errno));
        rc = -1;
    }
    if (rc != 0) {
        unlink(rw->tmp_path);
        rewrite_free(rw);
        return NULL;
    }
    jinn_dir_fsync(rw->path);
    if (old_fp) fclose(old_fp);
    FILE *nf = fopen(rw->path, "r+b");
    if (!nf) {
        fprintf(stderr, "jinn: reopen after rewrite of %s failed: %s\n", rw->path,
                strerror(errno));
        rewrite_free(rw);
        return NULL;
    }
    fseek(nf, 0, SEEK_END);
    rewrite_free(rw);
    return nf;
}
FILE *jinn_store_open_data(const char *path, int transient, int *created) {
    if (created) *created = 0;
    if (!path) {
        fprintf(stderr, "jinn: store: internal error — no path to open\n");
        exit(2);
    }
    if (!transient) {
        errno = 0;
        FILE *f = fopen(path, "r+b");
        if (f) return f;
        if (errno != ENOENT) {
            fprintf(stderr,
                    "jinn: store: cannot open %s for writing: %s — refusing to "
                    "recreate an existing store\n",
                    path, strerror(errno));
            exit(2);
        }
    }
    FILE *f = fopen(path, "w+b");
    if (!f) {
        fprintf(stderr, "jinn: store: cannot create %s: %s\n", path, strerror(errno));
        exit(2);
    }
    if (created) *created = 1;
    return f;
}
void jinn_store_finish_create(FILE *fp, const char *path) {
    if (!fp) return;
    if (fflush(fp) != 0) {
        fprintf(stderr, "jinn: store: writing the header of new store %s failed: %s\n",
                path ? path : "(store)", strerror(errno));
        exit(2);
    }
    (void)jinn_fsync_checked(fileno(fp), path);
    (void)jinn_dir_fsync(path);
}
void jinn_store_drop_indexes(const char *store_path) {
    if (!store_path) return;
    char dirbuf[4096];
    const char *slash = strrchr(store_path, '/');
    const char *dir = ".";
    const char *base = store_path;
    if (slash) {
        size_t n = (size_t)(slash - store_path);
        if (n == 0 || n >= sizeof(dirbuf)) return;
        memcpy(dirbuf, store_path, n);
        dirbuf[n] = '\0';
        dir = dirbuf;
        base = slash + 1;
    }
    size_t blen = strlen(base);
    if (blen > 6 && strcmp(base + blen - 6, ".store") == 0) blen -= 6;
    DIR *d = opendir(dir);
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        size_t nlen = strlen(e->d_name);
        if (nlen > blen + 5 && strncmp(e->d_name, base, blen) == 0 && e->d_name[blen] == '.' &&
            (strcmp(e->d_name + nlen - 4, ".idx") == 0 ||
             strcmp(e->d_name + nlen - 4, ".fts") == 0)) {
            char full[4600];
            snprintf(full, sizeof full, "%s/%s", dir, e->d_name);
            unlink(full);
        }
    }
    closedir(d);
}
#define JINN_STORE_LOCK_MAX 64
typedef struct {
    char            path[256];
    _Atomic(void *) owner;
    int32_t         depth;
    int             used;
} JinnStoreLock;
static JinnStoreLock g_store_locks[JINN_STORE_LOCK_MAX];
static pthread_mutex_t g_store_lock_table = PTHREAD_MUTEX_INITIALIZER;

static JinnStoreLock *store_lock_slot(const char *path) {
    if (!path) return NULL;
    pthread_mutex_lock(&g_store_lock_table);
    for (int i = 0; i < JINN_STORE_LOCK_MAX; i++) {
        if (g_store_locks[i].used && strcmp(g_store_locks[i].path, path) == 0) {
            pthread_mutex_unlock(&g_store_lock_table);
            return &g_store_locks[i];
        }
    }
    for (int i = 0; i < JINN_STORE_LOCK_MAX; i++) {
        if (!g_store_locks[i].used) {
            snprintf(g_store_locks[i].path, sizeof g_store_locks[i].path, "%s", path);
            g_store_locks[i].used = 1;
            g_store_locks[i].depth = 0;
            atomic_store(&g_store_locks[i].owner, NULL);
            pthread_mutex_unlock(&g_store_lock_table);
            return &g_store_locks[i];
        }
    }
    pthread_mutex_unlock(&g_store_lock_table);
    fprintf(stderr,
            "jinn: more than %d distinct stores opened for writing; the writer lock "
            "table is full and writes to '%s' are no longer serialised\n",
            JINN_STORE_LOCK_MAX, path);
    return NULL;
}
__attribute__((weak)) void jinn_sched_yield(void);
__attribute__((weak)) jinn_coro_t *jinn_current_coro(void);

static void store_lock_backoff(void) {
    if (jinn_sched_yield) {
        jinn_sched_yield();
        return;
    }
    struct timespec ns = {0, 10000};
    nanosleep(&ns, NULL);
}
static void *store_lock_self(void) {
    static _Thread_local char tl_lock_id;
    if (jinn_current_coro) {
        jinn_coro_t *c = jinn_current_coro();
        if (c) return c;
    }
    return &tl_lock_id;
}
void jinn_store_wlock(const char *path) {
    JinnStoreLock *slot = store_lock_slot(path);
    if (!slot) return;
    void *self = store_lock_self();
    if (atomic_load_explicit(&slot->owner, memory_order_relaxed) == self) {
        slot->depth++;
        return;
    }
    for (;;) {
        void *expected = NULL;
        if (atomic_compare_exchange_weak_explicit(&slot->owner, &expected, self,
                                                  memory_order_acquire,
                                                  memory_order_relaxed)) {
            slot->depth = 1;
            return;
        }
        store_lock_backoff();
    }
}
void jinn_store_wunlock(const char *path) {
    JinnStoreLock *slot = store_lock_slot(path);
    if (!slot) return;
    if (atomic_load_explicit(&slot->owner, memory_order_relaxed) != store_lock_self()) {
        return;
    }
    if (--slot->depth <= 0) {
        slot->depth = 0;
        atomic_store_explicit(&slot->owner, NULL, memory_order_release);
    }
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
