#define _GNU_SOURCE
#include "jinn_rt.h"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static const char *fault;
static long target, calls;
static int crash_mode, armed;

static int hit(const char *name) {
    if (!armed || strcmp(fault, name)) return 0;
    if (++calls != target) return 0;
    if (crash_mode == 1) _exit(90);
    errno = ENOSPC;
    return 1;
}
static void after(const char *name) {
    if (armed && crash_mode == 2 && !strcmp(fault, name) && calls == target) _exit(90);
}
size_t __real_fwrite(const void *, size_t, size_t, FILE *);
size_t __wrap_fwrite(const void *p, size_t s, size_t n, FILE *f) {
    int fail = hit("fwrite");
    if (fail && crash_mode != 2) {
        if (n > 1) return __real_fwrite(p, s, n - 1, f);
        return 0;
    }
    size_t r = __real_fwrite(p, s, n, f);
    after("fwrite");
    return r;
}
int __real_fflush(FILE *);
int __wrap_fflush(FILE *f) {
    if (hit("fflush") && crash_mode != 2) return EOF;
    int r = __real_fflush(f);
    after("fflush");
    return r;
}
int __real_fsync(int);
int __wrap_fsync(int fd) {
    if (hit("fsync") && crash_mode != 2) return -1;
    int r = __real_fsync(fd);
    after("fsync");
    return r;
}
int __real_fdatasync(int);
int __wrap_fdatasync(int fd) {
    if (hit("fdatasync") && crash_mode != 2) return -1;
    int r = __real_fdatasync(fd);
    after("fdatasync");
    return r;
}
FILE *__real_fopen(const char *, const char *);
FILE *__wrap_fopen(const char *p, const char *m) {
    if (hit("fopen") && crash_mode != 2) return NULL;
    FILE *r = __real_fopen(p, m);
    after("fopen");
    return r;
}
int __real_rename(const char *, const char *);
int __wrap_rename(const char *a, const char *b) {
    if (hit("rename") && crash_mode != 2) return -1;
    int r = __real_rename(a, b);
    after("rename");
    return r;
}
int __real_ftruncate(int, off_t);
int __wrap_ftruncate(int fd, off_t n) {
    if (hit("ftruncate") && crash_mode != 2) return -1;
    int r = __real_ftruncate(fd, n);
    after("ftruncate");
    return r;
}
int __real_unlink(const char *);
int __wrap_unlink(const char *p) {
    if (hit("unlink") && crash_mode != 2) return -1;
    int r = __real_unlink(p);
    after("unlink");
    return r;
}
jinn_coro_t *jinn_current_coro(void) { return NULL; }
uint64_t jinn_fnv1a(const void *data, int64_t len) {
    const unsigned char *p = data;
    uint64_t h = UINT64_C(14695981039346656037);
    for (int64_t i = 0; i < len; i++) h = (h ^ p[i]) * UINT64_C(1099511628211);
    return h;
}
static void put(FILE *fp, int64_t value) {
    jinn_store_fseek(fp, 0, SEEK_SET);
    jinn_store_fwrite(&value, 8, 1, fp);
    jinn_store_fflush(fp);
}
static int64_t get(FILE *fp) {
    int64_t n;
    jinn_store_fseek(fp, 0, SEEK_SET);
    jinn_store_fread(&n, 8, 1, fp);
    return n;
}
static FILE *data_open(const char *path, int init) {
    jinn_persistence_check_path(path);
    FILE *fp = fopen(path, init ? "w+b" : "r+b");
    if (!fp) jinn_persistence_fail("fixture data open");
    if (init) {
        put(fp, 1);
        jinn_store_finish_create(fp, path);
    }
    return fp;
}
int main(int argc, char **argv) {
    if (argc < 2) return 2;
    if (!strcmp(argv[1], "path")) {
        if (argc != 3) return 2;
        jinn_persistence_check_path(argv[2]);
        return 0;
    }
    if (!strcmp(argv[1], "cwd")) {
        jinn_persistence_prepare();
        if (chdir("nested") != 0) return 2;
        jinn_persistence_prepare();
        return 0;
    }
    if (!strcmp(argv[1], "hold")) {
        jinn_persistence_prepare();
        puts("ready");
        fflush(stdout);
        sleep(60);
        return 0;
    }
    int init = !strcmp(argv[1], "init");
    FILE *a = data_open("a.store", init), *b = data_open("b.store", init);
    FILE *wa = jinn_wal_open("a.wal"), *wb = jinn_wal_open("b.wal");
    JinnKV *kv = jinn_kv_open("state.kv");
    JinnBloom *bloom = jinn_bloom_open("state.bloom", 128);
    JinnVec *vec = jinn_vec_open("state.vec", 2);
    JinnCol *col = jinn_col_open("state.col", 8);
    JinnFts *fts = jinn_fts_open("state.fts");
    JinnIndex *idx = jinn_idx_open("state.idx");
    FILE *ver = jinn_ver_open("state.ver");
    if (!kv || !wa || !wb) jinn_persistence_fail("fixture handles");
    if (init) {
        int64_t n = 1;
        double point[] = {1, 2};
        jinn_kv_set(kv, "key", 3, 1);
        jinn_bloom_add_i64(bloom, 1);
        jinn_vec_insert(vec, point);
        jinn_col_append(col, &n);
        jinn_fts_add(fts, 1, "old", 3);
        jinn_idx_insert(idx, 11, 1);
        jinn_ver_append(ver, 1, 1, &n, 8);
    } else if (!strcmp(argv[1], "verify")) {
        int64_t n = get(a), other = get(b);
        int64_t expected = n == 1 ? 1 : 2;
        int64_t value = 0;
        if ((n != 1 && n != 2) || other != n || jinn_kv_get(kv, "key", 3) != n ||
            jinn_vec_count(vec) != expected || jinn_col_count(col) != expected ||
            jinn_fts_posting_count(fts) != expected || jinn_ver_count(ver, 1, 8) != expected ||
            jinn_idx_lookup(idx, 22) != (n == 1 ? -1 : 2) ||
            jinn_bloom_test_i64(bloom, 2) != (n == 2) ||
            !jinn_ver_at(ver, 1, n, &value, 8) || value != n) return 3;
        printf("%lld\n", (long long)n);
    } else {
        fault = argc > 2 ? argv[2] : "none";
        target = argc > 3 ? strtol(argv[3], NULL, 10) : 0;
        crash_mode = argc > 4 ? atoi(argv[4]) : 0;
        jinn_txn_begin();
        armed = 1;
        jinn_txn_track_store(&a, wa, "a.store");
        put(a, 2);
        jinn_txn_track_store(&b, wb, "b.store");
        put(b, 2);
        int64_t n = 2;
        double point[] = {2, 3};
        jinn_kv_set(kv, "key", 3, 2);
        jinn_bloom_add_i64(bloom, 2);
        jinn_vec_insert(vec, point);
        jinn_col_append(col, &n);
        jinn_fts_add(fts, 2, "new", 3);
        jinn_idx_insert(idx, 22, 2);
        jinn_ver_append(ver, 1, 2, &n, 8);
        if (!strcmp(argv[1], "rollback")) jinn_txn_rollback();
        else jinn_txn_commit();
        armed = 0;
        printf("ack %ld\n", calls);
    }
    jinn_kv_close(kv);
    jinn_bloom_close(bloom);
    jinn_vec_close(vec);
    jinn_col_close(col);
    jinn_fts_close(fts);
    jinn_idx_close(idx);
    jinn_ver_close(ver);
    jinn_wal_close(wa);
    jinn_wal_close(wb);
    fclose(a);
    fclose(b);
    return 0;
}
