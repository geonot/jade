/*
 * Crash-injection harness for the atomic durable write discipline
 * (task 8-21). Compiled and run by tests/crash_safety.rs.
 *
 * Three probes:
 *   1. fill-crash: a child dies at N points inside the fill callback of
 *      jinn_atomic_rewrite; the target file must always hold the
 *      complete OLD image afterward (never a mixture, never truncated).
 *   2. kv-churn: children mutate a JinnKV in a loop and are SIGKILLed at
 *      random times; after every kill the file must be a complete,
 *      self-consistent image (magic + header count == entries on disk ==
 *      count reported after reload).
 *   3. lock: a second writer on the same path must be refused while the
 *      first holds the store open, and admitted after it closes.
 *
 * Exit 0 on success; nonzero with a message on the first violation.
 */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include "jinn_rt.h"

#define DIE(...) do { fprintf(stderr, __VA_ARGS__); exit(1); } while (0)

static const char OLD_IMG[] = "OLD-STATE-0123456789-OLD-STATE";
static const char NEW_IMG[] = "NEW-STATE-abcdefghij-NEW-STATE-LONGER-THAN-OLD";

static void write_file(const char *path, const void *data, size_t len) {
    FILE *f = fopen(path, "wb");
    if (!f) DIE("cannot create %s\n", path);
    fwrite(data, 1, len, f);
    fclose(f);
}

static long read_file(const char *path, char *buf, size_t cap) {
    FILE *f = fopen(path, "rb");
    if (!f) return -1;
    long n = (long)fread(buf, 1, cap, f);
    fclose(f);
    return n;
}

/* ── Probe 1: die mid-fill at every prefix length ─────────────────── */

typedef struct {
    int crash_after; /* bytes of NEW_IMG to write before _exit */
} FillCrash;

static int crashing_fill(FILE *tmp, void *arg) {
    FillCrash *fc = (FillCrash *)arg;
    fwrite(NEW_IMG, 1, (size_t)fc->crash_after, tmp);
    fflush(tmp);
    _exit(42); /* simulated crash mid-rewrite */
}

static int full_fill(FILE *tmp, void *arg) {
    (void)arg;
    return fwrite(NEW_IMG, 1, sizeof(NEW_IMG), tmp) == sizeof(NEW_IMG) ? 0 : -1;
}

static void probe_fill_crash(const char *dir) {
    char path[512];
    snprintf(path, sizeof path, "%s/fill.dat", dir);

    for (int n = 0; n <= (int)sizeof(NEW_IMG); n += 7) {
        write_file(path, OLD_IMG, sizeof(OLD_IMG));
        pid_t pid = fork();
        if (pid == 0) {
            FillCrash fc = { n };
            jinn_atomic_rewrite(path, crashing_fill, &fc);
            _exit(0); /* unreachable */
        }
        int st = 0;
        waitpid(pid, &st, 0);
        char buf[256];
        long got = read_file(path, buf, sizeof buf);
        if (got != (long)sizeof(OLD_IMG) || memcmp(buf, OLD_IMG, sizeof(OLD_IMG)) != 0) {
            DIE("fill-crash@%d: target no longer holds the old image (%ld bytes)\n",
                n, got);
        }
    }

    /* And a completed rewrite must yield exactly the new image. */
    if (jinn_atomic_rewrite(path, full_fill, NULL) != 0) DIE("full rewrite failed\n");
    char buf[256];
    long got = read_file(path, buf, sizeof buf);
    if (got != (long)sizeof(NEW_IMG) || memcmp(buf, NEW_IMG, sizeof(NEW_IMG)) != 0) {
        DIE("completed rewrite: wrong content (%ld bytes)\n", got);
    }
    printf("fill-crash: ok\n");
}

/* ── Probe 2: SIGKILL a kv-churning child, verify image coherence ── */

#define KV_SLOT_BYTES 280 /* hash(8)+key(256)+value(8)+status(8) */

static void kv_verify(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) DIE("kv file missing after kill\n");
    char magic[8];
    int64_t count = -1;
    if (fread(magic, 1, 8, f) != 8 || memcmp(magic, "JINNKV\0\0", 8) != 0) {
        DIE("kv: bad magic after kill\n");
    }
    if (fread(&count, 8, 1, f) != 1 || count < 0) DIE("kv: bad header count\n");
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fclose(f);
    if (size != 16 + count * KV_SLOT_BYTES) {
        DIE("kv: mixed image: header says %lld entries but file is %ld bytes\n",
            (long long)count, size);
    }
    JinnKV *kv = jinn_kv_open(path);
    if (!kv) DIE("kv: reopen failed after kill\n");
    if (jinn_kv_count(kv) != count) {
        DIE("kv: reload count %lld != header %lld\n",
            (long long)jinn_kv_count(kv), (long long)count);
    }
    jinn_kv_close(kv);
}

static void probe_kv_churn(const char *dir, int rounds) {
    char path[512];
    snprintf(path, sizeof path, "%s/churn.kv", dir);

    for (int r = 0; r < rounds; r++) {
        pid_t pid = fork();
        if (pid == 0) {
            JinnKV *kv = jinn_kv_open(path);
            if (!kv) _exit(3);
            char key[32];
            for (int i = 0;; i++) {
                snprintf(key, sizeof key, "k%d", i % 37);
                if (i % 5 == 4) {
                    jinn_kv_del(kv, key, (int64_t)strlen(key));
                } else {
                    jinn_kv_set(kv, key, (int64_t)strlen(key), i);
                }
            }
            _exit(0);
        }
        usleep(1000 + (useconds_t)(rand() % 20000));
        kill(pid, SIGKILL);
        waitpid(pid, NULL, 0);
        kv_verify(path);
    }
    printf("kv-churn: ok (%d kills)\n", rounds);
}

/* ── Probe 3: single-writer lock ──────────────────────────────────── */

static void probe_lock(const char *dir) {
    char path[512];
    snprintf(path, sizeof path, "%s/lock.kv", dir);

    JinnKV *a = jinn_kv_open(path);
    if (!a) DIE("lock: first open failed\n");
    /* flock is per open-file-description, so a second open in this same
     * process conflicts exactly as a second process would. */
    fprintf(stderr, "(expected contention diagnostic follows)\n");
    JinnKV *b = jinn_kv_open(path);
    if (b) DIE("lock: second concurrent writer was admitted\n");
    jinn_kv_close(a);
    JinnKV *c = jinn_kv_open(path);
    if (!c) DIE("lock: reopen after close refused\n");
    jinn_kv_close(c);
    printf("lock: ok\n");
}

int main(int argc, char **argv) {
    const char *dir = argc > 1 ? argv[1] : ".";
    int rounds = argc > 2 ? atoi(argv[2]) : 25;
    srand((unsigned)getpid());
    probe_fill_crash(dir);
    probe_kv_churn(dir, rounds);
    probe_lock(dir);
    printf("durable-crash: all probes passed\n");
    return 0;
}
