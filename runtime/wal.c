#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <errno.h>
#include <limits.h>
#include <unistd.h>
#include "jinn_rt.h"
#define JINN_WAL_SYNC_NONE       0
#define JINN_WAL_SYNC_FDATASYNC  1
#define JINN_WAL_SYNC_FSYNC      2
#define JINN_WAL_SYNC_GROUP      3
static int jinn_wal_env_policy = -1;
static pthread_once_t wal_policy_once = PTHREAD_ONCE_INIT;
static void jinn_wal_init_env_policy(void) {
    const char *env = getenv("JINN_WAL_SYNC");
    if (!env || !*env) {
        jinn_wal_env_policy = -1;
    } else if (strcmp(env, "none") == 0) {
        jinn_wal_env_policy = JINN_WAL_SYNC_NONE;
    } else if (strcmp(env, "fsync") == 0) {
        jinn_wal_env_policy = JINN_WAL_SYNC_FSYNC;
    } else if (strcmp(env, "group") == 0) {
        jinn_wal_env_policy = JINN_WAL_SYNC_GROUP;
    } else {
        if (strcmp(env, "fdatasync") != 0) {
            fprintf(stderr,
                    "jinn: wal: unrecognised JINN_WAL_SYNC value '%s' — treating it "
                    "as 'fdatasync' (valid: none, fdatasync, fsync, group); note that "
                    "any set value overrides per-store durability decorators\n",
                    env);
        }
        jinn_wal_env_policy = JINN_WAL_SYNC_FDATASYNC;
    }
}
static int jinn_wal_get_env_policy(void) {
    pthread_once(&wal_policy_once, jinn_wal_init_env_policy);
    return jinn_wal_env_policy;
}
static int jinn_wal_get_policy(void) {
    int env = jinn_wal_get_env_policy();
    return env >= 0 ? env : JINN_WAL_SYNC_FDATASYNC;
}

#define JINN_WAL_POLICY_SLOTS 128
static struct {
    FILE *wal;
    int   policy;
} jinn_wal_policies[JINN_WAL_POLICY_SLOTS];
static _Atomic(int32_t) jinn_wal_policy_lock = 0;
static void jinn_wal_policy_acquire(void) {
    while (atomic_exchange_explicit(&jinn_wal_policy_lock, 1, memory_order_acquire) != 0) {
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}
static void jinn_wal_policy_release(void) {
    atomic_store_explicit(&jinn_wal_policy_lock, 0, memory_order_release);
}
void jinn_wal_set_policy(FILE *wal, int policy) {
    if (!wal) return;
    int stored = 0;
    jinn_wal_policy_acquire();
    for (int i = 0; i < JINN_WAL_POLICY_SLOTS; i++) {
        if (jinn_wal_policies[i].wal == wal || jinn_wal_policies[i].wal == NULL) {
            jinn_wal_policies[i].wal = wal;
            jinn_wal_policies[i].policy = policy;
            stored = 1;
            break;
        }
    }
    jinn_wal_policy_release();
    if (!stored) {
        fprintf(stderr,
                "jinn: wal: more than %d stores with durability decorators are open; "
                "this store's decorator is ignored and it falls back to per-record "
                "fdatasync\n",
                JINN_WAL_POLICY_SLOTS);
    }
}
static int jinn_wal_effective_policy(FILE *wal) {
    int env = jinn_wal_get_env_policy();
    if (env >= 0) return env;
    int policy = JINN_WAL_SYNC_FDATASYNC;
    jinn_wal_policy_acquire();
    for (int i = 0; i < JINN_WAL_POLICY_SLOTS; i++) {
        if (jinn_wal_policies[i].wal == NULL) break;
        if (jinn_wal_policies[i].wal == wal) {
            policy = jinn_wal_policies[i].policy;
            break;
        }
    }
    jinn_wal_policy_release();
    return policy;
}
static int jinn_wal_force(FILE *wal, int policy) {
    if (!wal) return -1;
    if (fflush(wal) != 0) return -1;
    int fd = fileno(wal);
    if (fd < 0) return -1;
    switch (policy) {
        case JINN_WAL_SYNC_NONE:
        case JINN_WAL_SYNC_GROUP:
            return 0;
        case JINN_WAL_SYNC_FSYNC:
            return fsync(fd) == 0 ? 0 : -1;
        case JINN_WAL_SYNC_FDATASYNC:
        default:
#if defined(__linux__)
            return fdatasync(fd) == 0 ? 0 : -1;
#else
            return fsync(fd) == 0 ? 0 : -1;
#endif
    }
}
void jinn_wal_commit_group(FILE *wal) {
    jinn_persistence_sync(wal);
}
static const char WAL_MAGIC[8]    = {'J','I','N','N','W','A','L','2'};
static const char WAL_MAGIC_V1[8] = {'J','I','N','N','W','A','L','\0'};
static uint32_t crc32_table[256];
static pthread_once_t crc32_once = PTHREAD_ONCE_INIT;
static void crc32_init(void) {
    for (uint32_t i = 0; i < 256; i++) {
        uint32_t c = i;
        for (int j = 0; j < 8; j++) {
            c = (c & 1) ? (0xEDB88320u ^ (c >> 1)) : (c >> 1);
        }
        crc32_table[i] = c;
    }
}

#define JINN_CRC_SEED 0xFFFFFFFFu
static uint32_t crc32_update(uint32_t crc, const void *data, size_t len) {
    pthread_once(&crc32_once, crc32_init);
    const uint8_t *p = (const uint8_t *)data;
    for (size_t i = 0; i < len; i++) {
        crc = crc32_table[(crc ^ p[i]) & 0xFF] ^ (crc >> 8);
    }
    return crc;
}
static uint32_t wal_frame_crc(uint32_t payload_len, uint8_t op, int64_t ts,
                              const void *payload) {
    uint32_t c = JINN_CRC_SEED;
    c = crc32_update(c, &payload_len, 4);
    c = crc32_update(c, &op, 1);
    c = crc32_update(c, &ts, 8);
    if (payload_len > 0 && payload) c = crc32_update(c, payload, payload_len);
    return c ^ 0xFFFFFFFFu;
}

static int wal_frame_len_ok(uint8_t op, uint32_t payload_len) {
    if (op == JINN_WAL_OP_TXN_BEGIN) return 1;
    return payload_len <= 64u * 1024 * 1024;
}
static long wal_scan_valid_end(FILE *f, int *damaged) {
    if (fseek(f, 0, SEEK_END) != 0) jinn_persistence_fail("WAL scan seek");
    long file_end = ftell(f);
    long valid_end = 8;
    if (file_end < 8 || fseek(f, 8, SEEK_SET) != 0)
        jinn_persistence_fail("WAL scan position");
    *damaged = 0;
    while (ftell(f) < file_end) {
        uint32_t payload_len;
        uint8_t  op;
        int64_t  ts;
        if (fread(&payload_len, 4, 1, f) != 1 ||
            fread(&op, 1, 1, f) != 1 ||
            fread(&ts, 8, 1, f) != 1) { *damaged = 1; break; }
        if (!wal_frame_len_ok(op, payload_len)) { *damaged = 1; break; }
        long body = ftell(f);
        if (file_end - body < (long)payload_len + 4) { *damaged = 1; break; }
        uint32_t c = JINN_CRC_SEED;
        c = crc32_update(c, &payload_len, 4);
        c = crc32_update(c, &op, 1);
        c = crc32_update(c, &ts, 8);
        uint8_t buf[4096];
        uint32_t left = payload_len;
        while (left > 0) {
            size_t chunk = left < sizeof buf ? left : sizeof buf;
            if (fread(buf, 1, chunk, f) != chunk) { *damaged = 1; goto done; }
            c = crc32_update(c, buf, chunk);
            left -= (uint32_t)chunk;
        }
        uint32_t stored;
        if (fread(&stored, 4, 1, f) != 1) { *damaged = 1; break; }
        if ((c ^ 0xFFFFFFFFu) != stored) { *damaged = 1; break; }
        valid_end = ftell(f);
    }
done:
    if (ferror(f)) jinn_persistence_fail("WAL scan read");
    return valid_end;
}
FILE *jinn_store_wal_open(const char *path) {
    jinn_persistence_check_path(path);
    return jinn_wal_open(path);
}
FILE *jinn_wal_open(const char *path) {
    FILE *f = fopen(path, "r+b");
    if (!f && errno != ENOENT) return NULL;
    if (f) {
        char magic[8];
        size_t got = fread(magic, 1, 8, f);
        if (got == 8 && memcmp(magic, WAL_MAGIC, 8) == 0) {
            int damaged = 0;
            long valid_end = wal_scan_valid_end(f, &damaged);
            if (damaged) {
                fprintf(stderr,
                        "jinn: wal: %s has a torn or corrupt tail; truncating to "
                        "last valid entry (offset %ld) before appending\n",
                        path, valid_end);
                if (ftruncate(fileno(f), (off_t)valid_end) != 0) {
                    fprintf(stderr, "jinn: wal: truncate of %s failed\n", path);
                    fclose(f);
                    return NULL;
                }
            }
            if (fseek(f, valid_end, SEEK_SET) != 0) {
                fclose(f);
                return NULL;
            }
            if (damaged) jinn_persistence_sync(f);
            return f;
        }
        if (got == 8 && memcmp(magic, WAL_MAGIC_V1, 8) == 0) {
            fprintf(stderr,
                    "jinn: wal: %s uses the v1 format (pre-8-22); upgrading by "
                    "truncation — v1 logs are redundant with the data file\n",
                    path);
            if (ftruncate(fileno(f), 0) != 0) {
                fclose(f);
                return NULL;
            }
            if (fseek(f, 0, SEEK_SET) != 0 || fwrite(WAL_MAGIC, 1, 8, f) != 8) {
                fclose(f);
                return NULL;
            }
            if (jinn_wal_force(f, JINN_WAL_SYNC_FDATASYNC) != 0) {
                fclose(f);
                return NULL;
            }
            return f;
        }
        if (got > 0) {
            fprintf(stderr,
                    "jinn: wal: %s is not a Jinn WAL (bad magic) — refusing to "
                    "touch it; move it aside to proceed\n",
                    path);
            fclose(f);
            exit(2);
        }
        fclose(f);
    }
    f = fopen(path, "w+b");
    if (!f) return NULL;
    if (fwrite(WAL_MAGIC, 1, 8, f) != 8) {
        fclose(f);
        return NULL;
    }
    if (jinn_wal_force(f, jinn_wal_get_policy() == JINN_WAL_SYNC_NONE
                              ? JINN_WAL_SYNC_NONE
                              : JINN_WAL_SYNC_FDATASYNC) != 0) {
        fprintf(stderr, "jinn: wal: cannot make new log %s durable\n", path);
        fclose(f);
        return NULL;
    }
    if (jinn_wal_get_policy() != JINN_WAL_SYNC_NONE) {
        if (jinn_dir_fsync(path) != 0) {
            fclose(f);
            return NULL;
        }
    }
    return f;
}
int jinn_wal_write(FILE *wal, uint8_t op, const void *payload, uint32_t payload_len) {
    if (!wal) return -1;
    int64_t ts = (int64_t)time(NULL);
    if (fseek(wal, 0, SEEK_END) != 0) {
        fprintf(stderr, "jinn: wal: fseek failed\n");
        return -1;
    }
    long start = ftell(wal);
    if (start < 0 || (payload_len && !payload)) return -1;
    uint32_t checksum = wal_frame_crc(payload_len, op, ts, payload);
    int ok = fwrite(&payload_len, 4, 1, wal) == 1 &&
             fwrite(&op, 1, 1, wal) == 1 &&
             fwrite(&ts, 8, 1, wal) == 1;
    if (ok && payload_len > 0 && payload) {
        ok = fwrite(payload, 1, payload_len, wal) == payload_len;
    }
    if (ok) ok = fwrite(&checksum, 4, 1, wal) == 1;
    if (!ok) {
        fprintf(stderr, "jinn: wal: append failed; truncating partial frame\n");
        fflush(wal);
        if (start >= 0) (void)ftruncate(fileno(wal), (off_t)start);
        fseek(wal, 0, SEEK_END);
        return -1;
    }
    if (jinn_wal_force(wal, jinn_wal_effective_policy(wal)) != 0) {
        fprintf(stderr, "jinn: wal: sync failed — record may not be durable\n");
        return -1;
    }
    return 0;
}
void jinn_wal_write_must(FILE *wal, uint8_t op, const void *payload,
                         uint32_t payload_len) {
    if (jinn_wal_write(wal, op, payload, payload_len) != 0) {
        fprintf(stderr, "jinn: wal: cannot guarantee durability — aborting\n");
        abort();
    }
}
void jinn_wal_checkpoint(FILE *wal) {
    if (!wal || jinn_txn_active()) return;
    jinn_persistence_sync(wal);
    if (ftruncate(fileno(wal), 8) != 0 || fseek(wal, 8, SEEK_SET) != 0)
        jinn_persistence_fail("WAL checkpoint truncation");
    jinn_persistence_sync(wal);
}

void jinn_store_save(FILE *fp, FILE *wal) {
    if (fp) jinn_persistence_sync(fp);
    jinn_wal_checkpoint(wal);
}

void jinn_wal_close(FILE *wal) {
    if (!wal) return;
    jinn_wal_policy_acquire();
    for (int i = 0; i < JINN_WAL_POLICY_SLOTS; i++) {
        if (jinn_wal_policies[i].wal == wal) {
            for (int j = i; j + 1 < JINN_WAL_POLICY_SLOTS; j++) {
                jinn_wal_policies[j] = jinn_wal_policies[j + 1];
                if (jinn_wal_policies[j].wal == NULL) break;
            }
            jinn_wal_policies[JINN_WAL_POLICY_SLOTS - 1].wal = NULL;
            break;
        }
    }
    jinn_wal_policy_release();
    if (fclose(wal) != 0) jinn_persistence_fail("WAL close");
}
typedef struct JinnTxnFile {
    FILE  *fp;
    FILE **fpp;
    FILE  *wal;
    char  *path;
    long   wal_off;
    unsigned char *snap;
    long   snap_len;
    int    mem_only;
    int    disk_only;
    void (*on_rollback)(void *);
    void (*on_release)(void *);
    void  *rb_arg;
    struct JinnTxnFile *next;
} JinnTxnFile;
typedef struct JinnTxnState {
    int          depth;
    JinnTxnFile *files;
} JinnTxnState;
static _Thread_local JinnTxnState *tl_txn_fallback = NULL;
static JinnTxnState **jinn_txn_slot(void) {
    jinn_coro_t *c = jinn_current_coro();
    if (c) return (JinnTxnState **)&c->txn_state;
    return &tl_txn_fallback;
}
static JinnTxnState *jinn_txn_cur(void) {
    return *jinn_txn_slot();
}
int jinn_txn_active(void) {
    JinnTxnState *t = jinn_txn_cur();
    return t && t->depth > 0;
}
void jinn_txn_begin(void) {
    JinnTxnState **slot = jinn_txn_slot();
    if (!*slot || (*slot)->depth == 0) {
        jinn_store_writer_enter();
        jinn_persistence_prepare();
    }
    if (!*slot) {
        *slot = (JinnTxnState *)calloc(1, sizeof(JinnTxnState));
        if (!*slot) {
            fprintf(stderr, "jinn: txn: out of memory opening transaction\n");
            abort();
        }
    }
    (*slot)->depth++;
}
static JinnTxnFile *jinn_txn_find(JinnTxnState *st, FILE **fpp, FILE *fp) {
    for (JinnTxnFile *t = st->files; t; t = t->next) {
        if (fpp && t->fpp == fpp) return t;
        if (!fpp && !t->fpp && t->fp == fp) return t;
    }
    return NULL;
}
static void jinn_txn_free_files(JinnTxnState *st) {
    JinnTxnFile *t = st->files;
    while (t) {
        JinnTxnFile *n = t->next;
        if (t->on_release) t->on_release(t->rb_arg);
        free(t->snap);
        free(t->path);
        free(t);
        t = n;
    }
    st->files = NULL;
}
static void jinn_txn_release(void) {
    JinnTxnState **slot = jinn_txn_slot();
    if (*slot) {
        jinn_txn_free_files(*slot);
        free(*slot);
        *slot = NULL;
        jinn_store_writer_leave();
    }
}
static long jinn_txn_snapshot_max(void) {
    static long cached = -1;
    if (cached >= 0) return cached;
    const char *env = getenv("JINN_TXN_SNAPSHOT_MAX");
    long v = 256L * 1024 * 1024;
    if (env) {
        char *end;
        errno = 0;
        v = strtol(env, &end, 10);
        if (errno || !*env || *end || v <= 0)
            jinn_persistence_fail("JINN_TXN_SNAPSHOT_MAX must be a positive byte count");
    }
    cached = v;
    return cached;
}
static void jinn_txn_track_impl(FILE **fpp, FILE *fp, FILE *wal,
                                const char *path,
                                void (*cb)(void *), void *arg) {
    JinnTxnState *st = jinn_txn_cur();
    if (!st || st->depth <= 0) return;
    FILE *cur_fp = fpp ? *fpp : fp;
    if (!cur_fp) jinn_persistence_fail("transaction participant handle");
    if (jinn_txn_find(st, fpp, fp)) return;
    JinnTxnFile *t = (JinnTxnFile *)calloc(1, sizeof *t);
    if (!t) {
        fprintf(stderr, "jinn: txn: out of memory tracking store — aborting "
                        "(cannot guarantee rollback)\n");
        abort();
    }
    t->fp = fp;
    t->fpp = fpp;
    t->wal = wal;
    t->path = path ? strdup(path) : jinn_persistence_file_path(cur_fp);
    if (!t->path) jinn_persistence_fail("transaction path allocation");
    t->on_rollback = cb;
    t->rb_arg = arg;
    if (path) jinn_store_wlock(t->path);
    if (wal) {
        if (fflush(wal) != 0 || fseek(wal, 0, SEEK_END) != 0 ||
            (t->wal_off = ftell(wal)) < 0)
            jinn_persistence_fail("transaction WAL position");
    }
    if (fflush(cur_fp) != 0) jinn_persistence_fail("transaction snapshot flush");
    long cur = ftell(cur_fp);
    if (cur < 0 || fseek(cur_fp, 0, SEEK_END) != 0 || (t->snap_len = ftell(cur_fp)) < 0)
        jinn_persistence_fail("transaction snapshot position");
    if (t->snap_len > jinn_txn_snapshot_max()) {
        fprintf(stderr,
                "jinn: txn: store%s%s is %ld bytes, over the transaction "
                "snapshot limit of %ld; raise JINN_TXN_SNAPSHOT_MAX or move "
                "this mutation out of the transaction block\n",
                path ? " " : "", path ? path : "",
                t->snap_len, jinn_txn_snapshot_max());
        free(t->path);
        free(t);
        abort();
    }
    t->snap = (unsigned char *)malloc(t->snap_len > 0 ? (size_t)t->snap_len : 1);
    if (!t->snap) {
        fprintf(stderr, "jinn: txn: snapshot allocation failed — aborting\n");
        free(t->path);
        free(t);
        abort();
    }
    if (t->snap_len > 0) {
        if (fseek(cur_fp, 0, SEEK_SET) != 0 ||
            fread(t->snap, 1, (size_t)t->snap_len, cur_fp) != (size_t)t->snap_len) {
            fprintf(stderr, "jinn: txn: snapshot read failed — aborting\n");
            free(t->snap);
            free(t->path);
            free(t);
            abort();
        }
    }
    if (fseek(cur_fp, cur, SEEK_SET) != 0)
        jinn_persistence_fail("transaction snapshot seek");
    jinn_txn_journal_snapshot(t->path, t->snap, t->snap_len);
    if (wal) {
        jinn_txn_track_impl(NULL, wal, NULL, NULL, NULL, NULL);
        if ((uint64_t)t->snap_len > UINT32_MAX)
            jinn_persistence_fail("WAL undo image exceeds 4 GiB");
        jinn_wal_write_must(wal, JINN_WAL_OP_TXN_BEGIN, t->snap, (uint32_t)t->snap_len);
        jinn_persistence_sync(wal);
    }
    t->next = st->files;
    st->files = t;
}
void jinn_txn_track_store(FILE **fpp, FILE *wal, const char *path) {
    jinn_txn_track_impl(fpp, NULL, wal, path, NULL, NULL);
}
void jinn_txn_track_aux(FILE *fp, void (*cb)(void *), void *arg) {
    jinn_txn_track_impl(NULL, fp, NULL, NULL, cb, arg);
}
int jinn_txn_is_tracked(void *key) {
    JinnTxnState *st = jinn_txn_cur();
    if (!st || st->depth <= 0) return 0;
    return jinn_txn_find(st, NULL, (FILE *)key) != NULL;
}
static JinnTxnFile *jinn_txn_new_entry(void) {
    JinnTxnFile *t = (JinnTxnFile *)calloc(1, sizeof *t);
    if (!t) {
        fprintf(stderr, "jinn: txn: out of memory tracking store state — aborting "
                        "(cannot guarantee rollback)\n");
        abort();
    }
    return t;
}
void jinn_txn_track_mem(void *key, void (*rb)(void *), void (*rel)(void *), void *arg) {
    JinnTxnState *st = jinn_txn_cur();
    if (!st || st->depth <= 0) return;
    if (jinn_txn_find(st, NULL, (FILE *)key)) return;
    JinnTxnFile *t = jinn_txn_new_entry();
    t->fp = (FILE *)key;
    t->mem_only = 1;
    t->on_rollback = rb;
    t->on_release = rel;
    t->rb_arg = arg;
    t->next = st->files;
    st->files = t;
}
void jinn_txn_track_trunc(FILE *fp) {
    jinn_txn_track_aux(fp, NULL, NULL);
}
void jinn_txn_track_path(const char *path) {
    JinnTxnState *st = jinn_txn_cur();
    if (!st || st->depth <= 0) return;
    for (JinnTxnFile *t = st->files; t; t = t->next)
        if (t->disk_only && strcmp(t->path, path) == 0) return;
    FILE *fp = fopen(path, "r+b");
    if (!fp) jinn_persistence_fail("transaction sidecar open");
    jinn_txn_track_aux(fp, NULL, NULL);
    JinnTxnFile *t = st->files;
    free(t->path);
    t->path = strdup(path);
    if (!t->path) jinn_persistence_fail("transaction path allocation");
    t->disk_only = 1;
    t->fp = NULL;
    if (fclose(fp) != 0) jinn_persistence_fail("transaction sidecar close");
}
void jinn_txn_swap_fp(FILE *oldfp, FILE *newfp) {
    JinnTxnState *st = jinn_txn_cur();
    if (!st) return;
    for (JinnTxnFile *t = st->files; t; t = t->next) {
        if (!t->fpp && t->fp == oldfp) t->fp = newfp;
    }
}
void jinn_txn_commit(void) {
    JinnTxnState *st = jinn_txn_cur();
    if (!st || st->depth <= 0) return;
    if (--st->depth > 0) return;
    for (JinnTxnFile *t = st->files; t; t = t->next) {
        if (t->mem_only || t->disk_only) continue;
        jinn_persistence_sync(t->fpp ? *t->fpp : t->fp);
    }
    for (JinnTxnFile *t = st->files; t; t = t->next) {
        if (!t->wal) continue;
        jinn_wal_write_must(t->wal, JINN_WAL_OP_TXN_COMMIT, NULL, 0);
        jinn_persistence_sync(t->wal);
    }
    jinn_txn_journal_finish();
    for (JinnTxnFile *t = st->files; t; t = t->next)
        if (t->fpp && t->path) jinn_store_wunlock(t->path);
    jinn_txn_release();
}
static int txn_snap_fill(FILE *tmp, void *arg) {
    JinnTxnFile *t = (JinnTxnFile *)arg;
    if (t->snap_len > 0 &&
        fwrite(t->snap, 1, (size_t)t->snap_len, tmp) != (size_t)t->snap_len) {
        return -1;
    }
    return 0;
}
void jinn_txn_rollback(void) {
    JinnTxnState *st = jinn_txn_cur();
    if (!st || st->depth <= 0) return;
    st->depth = 0;
    for (JinnTxnFile *t = st->files; t; t = t->next) {
        if (t->mem_only) {
            if (t->on_rollback) t->on_rollback(t->rb_arg);
            continue;
        }
        if (t->fpp && t->path) {
            if (jinn_atomic_rewrite_reopen(t->path, txn_snap_fill, t, t->fpp) != 0)
                jinn_persistence_fail("transaction rollback rewrite");
        } else if (t->disk_only) {
            if (jinn_atomic_rewrite(t->path, txn_snap_fill, t) != 0)
                jinn_persistence_fail("transaction rollback sidecar rewrite");
        } else if (t->fp) {
            if (fseek(t->fp, 0, SEEK_SET) != 0 ||
                fwrite(t->snap, 1, (size_t)t->snap_len, t->fp) != (size_t)t->snap_len ||
                fflush(t->fp) != 0 || ftruncate(fileno(t->fp), (off_t)t->snap_len) != 0 ||
                fseek(t->fp, 0, SEEK_END) != 0)
                jinn_persistence_fail("transaction rollback restore");
            jinn_persistence_sync(t->fp);
        }
        if (t->on_rollback) t->on_rollback(t->rb_arg);
    }
    jinn_txn_journal_finish();
    for (JinnTxnFile *t = st->files; t; t = t->next)
        if (t->fpp && t->path) jinn_store_wunlock(t->path);
    jinn_txn_release();
}

int64_t jinn_wal_size(FILE *wal) {
    if (!wal) return 0;
    long cur = ftell(wal);
    fseek(wal, 0, SEEK_END);
    long end = ftell(wal);
    fseek(wal, cur, SEEK_SET);
    return (end > 8) ? (int64_t)(end - 8) : 0;
}
static _Thread_local int tl_wal_replay_complete = 1;
int jinn_wal_replay_was_complete(void) { return tl_wal_replay_complete; }
static long wal_stop_before_uncommitted(FILE *wal, long file_end,
                                        long *begin_off_out) {
    long pending_begin = -1;
    long valid_end = 8;
    fseek(wal, 8, SEEK_SET);
    while (ftell(wal) < file_end) {
        long frame_start = ftell(wal);
        uint32_t payload_len;
        uint8_t  op;
        int64_t  ts;
        if (fread(&payload_len, 4, 1, wal) != 1 ||
            fread(&op, 1, 1, wal) != 1 ||
            fread(&ts, 8, 1, wal) != 1) break;
        if (!wal_frame_len_ok(op, payload_len)) break;
        long body = ftell(wal);
        if (file_end - body < (long)payload_len + 4) break;
        uint32_t c = JINN_CRC_SEED;
        c = crc32_update(c, &payload_len, 4);
        c = crc32_update(c, &op, 1);
        c = crc32_update(c, &ts, 8);
        uint8_t buf[4096];
        uint32_t left = payload_len;
        int bad = 0;
        while (left > 0) {
            size_t chunk = left < sizeof buf ? left : sizeof buf;
            if (fread(buf, 1, chunk, wal) != chunk) { bad = 1; break; }
            c = crc32_update(c, buf, chunk);
            left -= (uint32_t)chunk;
        }
        if (bad) break;
        uint32_t stored;
        if (fread(&stored, 4, 1, wal) != 1) break;
        if ((c ^ 0xFFFFFFFFu) != stored) break;
        if (op == JINN_WAL_OP_TXN_BEGIN) {
            pending_begin = frame_start;
        } else if (op == JINN_WAL_OP_TXN_COMMIT) {
            pending_begin = -1;
        }
        valid_end = ftell(wal);
    }
    if (begin_off_out) *begin_off_out = pending_begin;
    return pending_begin >= 0 ? pending_begin : valid_end;
}
int jinn_wal_uncommitted_begin(FILE *wal, unsigned char **snap_out,
                               int64_t *len_out) {
    if (snap_out) *snap_out = NULL;
    if (len_out) *len_out = 0;
    if (!wal) return 0;
    long saved = ftell(wal);
    fseek(wal, 0, SEEK_END);
    long file_end = ftell(wal);
    long begin_off = -1;
    (void)wal_stop_before_uncommitted(wal, file_end, &begin_off);
    if (begin_off < 0) {
        fseek(wal, saved, SEEK_SET);
        return 0;
    }
    fseek(wal, begin_off, SEEK_SET);
    uint32_t payload_len;
    uint8_t  op;
    int64_t  ts;
    if (fread(&payload_len, 4, 1, wal) != 1 ||
        fread(&op, 1, 1, wal) != 1 ||
        fread(&ts, 8, 1, wal) != 1) {
        fseek(wal, saved, SEEK_SET);
        return 0;
    }
    unsigned char *buf = NULL;
    if (payload_len > 0) {
        buf = (unsigned char *)malloc(payload_len);
        if (!buf) jinn_persistence_fail("WAL undo allocation");
        if (fread(buf, 1, payload_len, wal) != payload_len) {
            jinn_persistence_fail("WAL undo read");
        }
    }
    if (snap_out) {
        *snap_out = buf;
    } else {
        free(buf);
    }
    if (len_out) *len_out = (int64_t)payload_len;
    fseek(wal, saved, SEEK_SET);
    return 1;
}
int64_t jinn_wal_replay(FILE *wal, jinn_wal_replay_cb callback, void *user_data) {
    if (!wal || !callback) return -1;
    tl_wal_replay_complete = 1;

    long saved = ftell(wal);
    fseek(wal, 0, SEEK_END);
    long file_end = ftell(wal);
    long stop_at = wal_stop_before_uncommitted(wal, file_end, NULL);
    fseek(wal, 8, SEEK_SET);
    int64_t count = 0;
    while (ftell(wal) < stop_at) {
        uint32_t payload_len;
        uint8_t  op;
        int64_t  ts;
        if (fread(&payload_len, 4, 1, wal) != 1) break;
        if (fread(&op, 1, 1, wal) != 1) break;
        if (fread(&ts, 8, 1, wal) != 1) break;
        if (!wal_frame_len_ok(op, payload_len)) break;
        if (op == JINN_WAL_OP_TXN_BEGIN || op == JINN_WAL_OP_TXN_COMMIT) {
            if (fseek(wal, (long)payload_len + 4, SEEK_CUR) != 0) break;
            continue;
        }
        long remaining = file_end - ftell(wal);
        if (remaining < (long)(payload_len + 4)) break;
        uint8_t *payload = NULL;
        if (payload_len > 0) {
            payload = (uint8_t *)malloc(payload_len);
            if (!payload) {
                tl_wal_replay_complete = 0;
                break;
            }
            if (fread(payload, 1, payload_len, wal) != payload_len) {
                free(payload);
                break;
            }
        }
        uint32_t stored_crc;
        if (fread(&stored_crc, 4, 1, wal) != 1) {
            free(payload);
            break;
        }
        uint32_t computed_crc = wal_frame_crc(payload_len, op, ts, payload);
        if (computed_crc != stored_crc) {
            free(payload);
            break;
        }
        callback(op, payload, payload_len, ts, user_data);
        free(payload);
        count++;
    }
    if (ftell(wal) != stop_at || ferror(wal)) tl_wal_replay_complete = 0;
    if (fseek(wal, saved, SEEK_SET) != 0) tl_wal_replay_complete = 0;
    return count;
}
