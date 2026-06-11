/*
 * Jinn WAL (Write-Ahead Log) Runtime
 *
 * File format: [8B magic "JINNWAL\0"][entries...]
 * Entry:       [4B payload_len][1B op][8B timestamp][payload_len bytes][4B CRC32]
 *
 * Ops: 1=Insert, 2=Update, 3=Delete(soft), 4=Destroy(hard)
 *
 * Durability model:
 *   The WAL guarantees that every entry returned to user code as "committed"
 *   has been forced to stable storage via fdatasync(2) (Linux) or fsync(2)
 *   (macOS / where fdatasync is unavailable). After fdatasync returns, the
 *   data and minimum file metadata required to retrieve it survive an OS
 *   crash or power loss.
 *
 *   The sync policy is selected at runtime by the JINN_WAL_SYNC environment
 *   variable, parsed once on first WAL open:
 *     - "none"      : no sync; fastest but unsafe (test/bench only)
 *     - "fdatasync" : default; sync after every entry append
 *     - "fsync"     : full fsync after every entry append
 *     - "group"     : do not sync per-entry; caller must invoke
 *                     jinn_wal_commit_group() at transaction boundaries
 *
 *   Group-commit lets a higher-level coordinator amortize fsync latency
 *   across many records of one logical transaction. Records written between
 *   commits are NOT durable until commit_group() returns.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>
#include "jinn_rt.h"

/* Sync policy --------------------------------------------------------- */
#define JINN_WAL_SYNC_NONE       0
#define JINN_WAL_SYNC_FDATASYNC  1
#define JINN_WAL_SYNC_FSYNC      2
#define JINN_WAL_SYNC_GROUP      3

static int  jinn_wal_sync_policy = -1;

static int jinn_wal_get_policy(void) {
    if (jinn_wal_sync_policy >= 0) return jinn_wal_sync_policy;
    const char *env = getenv("JINN_WAL_SYNC");
    if (!env || !*env) {
        jinn_wal_sync_policy = JINN_WAL_SYNC_FDATASYNC;
    } else if (strcmp(env, "none") == 0) {
        jinn_wal_sync_policy = JINN_WAL_SYNC_NONE;
    } else if (strcmp(env, "fsync") == 0) {
        jinn_wal_sync_policy = JINN_WAL_SYNC_FSYNC;
    } else if (strcmp(env, "group") == 0) {
        jinn_wal_sync_policy = JINN_WAL_SYNC_GROUP;
    } else {
        /* Default for unknown values: fdatasync. */
        jinn_wal_sync_policy = JINN_WAL_SYNC_FDATASYNC;
    }
    return jinn_wal_sync_policy;
}

static void jinn_wal_force(FILE *wal, int policy) {
    if (!wal) return;
    /* Inside an open transaction every store runs in group-commit mode:
     * per-record syncs are deferred until jinn_txn_commit() issues the
     * group barrier (or rollback truncates the records away). */
    if (jinn_txn_active()) {
        fflush(wal);
        return;
    }
    /* Push libc buffers to the kernel first. */
    fflush(wal);
    int fd = fileno(wal);
    if (fd < 0) return;
    switch (policy) {
        case JINN_WAL_SYNC_NONE:
        case JINN_WAL_SYNC_GROUP:
            return;
        case JINN_WAL_SYNC_FSYNC:
            (void)fsync(fd);
            return;
        case JINN_WAL_SYNC_FDATASYNC:
        default:
#if defined(__linux__)
            if (fdatasync(fd) == 0) return;
#endif
            (void)fsync(fd);
            return;
    }
}

/* Public: explicit group-commit barrier. Always issues a full fdatasync
 * (or fsync where fdatasync is unavailable) regardless of the configured
 * default policy. Used by transaction coordinators to make a batch of
 * appended records durable. */
void jinn_wal_commit_group(FILE *wal) {
    if (!wal) return;
    fflush(wal);
    int fd = fileno(wal);
    if (fd < 0) return;
#if defined(__linux__)
    if (fdatasync(fd) == 0) return;
#endif
    (void)fsync(fd);
}

static const char WAL_MAGIC[8] = {'J','I','N','N','W','A','L','\0'};

/* Simple CRC32 (IEEE polynomial) */
static uint32_t crc32_table[256];
static int crc32_initialized = 0;

static void crc32_init(void) {
    if (crc32_initialized) return;
    for (uint32_t i = 0; i < 256; i++) {
        uint32_t c = i;
        for (int j = 0; j < 8; j++) {
            c = (c & 1) ? (0xEDB88320u ^ (c >> 1)) : (c >> 1);
        }
        crc32_table[i] = c;
    }
    crc32_initialized = 1;
}

static uint32_t crc32(const void *data, size_t len) {
    crc32_init();
    const uint8_t *p = (const uint8_t *)data;
    uint32_t crc = 0xFFFFFFFFu;
    for (size_t i = 0; i < len; i++) {
        crc = crc32_table[(crc ^ p[i]) & 0xFF] ^ (crc >> 8);
    }
    return crc ^ 0xFFFFFFFFu;
}

/* Open or create a WAL file. Returns FILE* or NULL. */
FILE *jinn_wal_open(const char *path) {
    FILE *f = fopen(path, "r+b");
    if (f) {
        /* Verify magic */
        char magic[8];
        if (fread(magic, 1, 8, f) == 8 && memcmp(magic, WAL_MAGIC, 8) == 0) {
            fseek(f, 0, SEEK_END);
            return f;
        }
        /* Bad magic — recreate */
        fclose(f);
    }
    f = fopen(path, "w+b");
    if (!f) return NULL;
    fwrite(WAL_MAGIC, 1, 8, f);
    /* The magic header itself must be durable so a torn create cannot
     * later be mistaken for a valid empty WAL with garbage entries. */
    jinn_wal_force(f, jinn_wal_get_policy() == JINN_WAL_SYNC_NONE
                          ? JINN_WAL_SYNC_NONE
                          : JINN_WAL_SYNC_FDATASYNC);
    return f;
}

/* Write a WAL entry.
 * op: 1=insert, 2=update, 3=delete, 4=destroy
 * payload: record bytes (for insert/update) or offset bytes (for delete/destroy)
 * payload_len: size of payload
 */
void jinn_wal_write(FILE *wal, uint8_t op, const void *payload, uint32_t payload_len) {
    if (!wal) return;

    int64_t ts = (int64_t)time(NULL);

    /* Seek to end */
    if (fseek(wal, 0, SEEK_END) != 0) {
        fprintf(stderr, "jinn: wal: fseek failed\n");
        return;
    }

    /* Write: [4B len][1B op][8B timestamp][payload][4B CRC32] */
    if (fwrite(&payload_len, 4, 1, wal) != 1 ||
        fwrite(&op, 1, 1, wal) != 1 ||
        fwrite(&ts, 8, 1, wal) != 1) {
        fprintf(stderr, "jinn: wal: write header failed\n");
        return;
    }
    if (payload_len > 0 && payload) {
        if (fwrite(payload, 1, payload_len, wal) != payload_len) {
            fprintf(stderr, "jinn: wal: write payload failed\n");
            return;
        }
    }

    /* CRC over op + timestamp + payload */
    size_t crc_len = 1 + 8 + payload_len;
    uint8_t *crc_buf = (uint8_t *)malloc(crc_len);
    if (crc_buf) {
        crc_buf[0] = op;
        memcpy(crc_buf + 1, &ts, 8);
        if (payload_len > 0 && payload) {
            memcpy(crc_buf + 9, payload, payload_len);
        }
        uint32_t checksum = crc32(crc_buf, crc_len);
        fwrite(&checksum, 4, 1, wal);
        free(crc_buf);
    } else {
        uint32_t zero = 0;
        fwrite(&zero, 4, 1, wal);
    }
    /* Per-record durability per JINN_WAL_SYNC. Group-commit policy defers
     * until jinn_wal_commit_group(). */
    jinn_wal_force(wal, jinn_wal_get_policy());
}

/* Checkpoint: truncate WAL back to just the magic header. */
void jinn_wal_checkpoint(FILE *wal) {
    if (!wal) return;
    /* Reopen as truncate — we can't just ftruncate portably, so rewrite magic */
    int fd = fileno(wal);
    if (fd >= 0) {
        if (ftruncate(fd, 8) == 0) {
            fseek(wal, 8, SEEK_SET);
        }
    }
    /* Checkpoint is a durability boundary: callers expect that on return,
     * the truncated state is on stable storage. Always force regardless of
     * the per-record sync policy (except explicit "none" for tests). */
    int policy = jinn_wal_get_policy();
    jinn_wal_force(wal, policy == JINN_WAL_SYNC_NONE
                            ? JINN_WAL_SYNC_NONE
                            : JINN_WAL_SYNC_FDATASYNC);
}

/* Close WAL file. */
void jinn_wal_close(FILE *wal) {
    if (wal) fclose(wal);
}

/* ── Transactions ──────────────────────────────────────────────────
 *
 * A `transaction` block makes the stores it touches atomic with respect
 * to escaping errors: `jinn_txn_begin()` opens the scope; every store
 * mutation that runs inside it registers its data file and WAL via
 * jinn_txn_track(), which snapshots the data file and records the WAL
 * offset on first contact. While a transaction is active, WAL writes
 * run in group-commit mode (no per-record fsync).
 *
 *   commit   — group-syncs each tracked WAL and fsyncs each data file;
 *              the batch becomes durable as a unit.
 *   rollback — restores each data file from its snapshot, truncates
 *              each WAL back to the recorded offset, and invokes any
 *              auxiliary rollback callback (e.g. index header reload)
 *              so in-memory handle state is discarded too.
 *
 * Nested begins are counted; only the outermost commit applies. A
 * rollback aborts the whole nest. Single-threaded by design: stores
 * are flock-guarded per process and transactions are lexical scopes.
 */

typedef struct JinnTxnFile {
    FILE *fp;
    FILE *wal;
    long  wal_off;
    unsigned char *snap;
    long  snap_len;
    void (*on_rollback)(void *);
    void *rb_arg;
    struct JinnTxnFile *next;
} JinnTxnFile;

static int          jinn_txn_depth = 0;
static JinnTxnFile *jinn_txn_files = NULL;

int jinn_txn_active(void) {
    return jinn_txn_depth > 0;
}

void jinn_txn_begin(void) {
    jinn_txn_depth++;
}

static JinnTxnFile *jinn_txn_find(FILE *fp) {
    for (JinnTxnFile *t = jinn_txn_files; t; t = t->next) {
        if (t->fp == fp) return t;
    }
    return NULL;
}

static void jinn_txn_free_files(void) {
    JinnTxnFile *t = jinn_txn_files;
    while (t) {
        JinnTxnFile *n = t->next;
        free(t->snap);
        free(t);
        t = n;
    }
    jinn_txn_files = NULL;
}

static void jinn_txn_track_impl(FILE *fp, FILE *wal,
                                void (*cb)(void *), void *arg) {
    if (jinn_txn_depth <= 0 || !fp) return;
    if (jinn_txn_find(fp)) return;
    JinnTxnFile *t = (JinnTxnFile *)calloc(1, sizeof *t);
    if (!t) return;
    t->fp = fp;
    t->wal = wal;
    t->on_rollback = cb;
    t->rb_arg = arg;
    if (wal) {
        fflush(wal);
        fseek(wal, 0, SEEK_END);
        t->wal_off = ftell(wal);
    }
    fflush(fp);
    long cur = ftell(fp);
    fseek(fp, 0, SEEK_END);
    t->snap_len = ftell(fp);
    if (t->snap_len < 0) t->snap_len = 0;
    t->snap = (unsigned char *)malloc(t->snap_len > 0 ? (size_t)t->snap_len : 1);
    if (!t->snap) {
        free(t);
        fseek(fp, cur, SEEK_SET);
        return;
    }
    if (t->snap_len > 0) {
        fseek(fp, 0, SEEK_SET);
        if (fread(t->snap, 1, (size_t)t->snap_len, fp) != (size_t)t->snap_len) {
            fprintf(stderr, "jinn: txn: snapshot read failed\n");
            free(t->snap);
            free(t);
            fseek(fp, cur, SEEK_SET);
            return;
        }
    }
    fseek(fp, cur, SEEK_SET);
    t->next = jinn_txn_files;
    jinn_txn_files = t;
}

void jinn_txn_track(FILE *fp, FILE *wal) {
    jinn_txn_track_impl(fp, wal, NULL, NULL);
}

void jinn_txn_track_aux(FILE *fp, void (*cb)(void *), void *arg) {
    jinn_txn_track_impl(fp, NULL, cb, arg);
}

void jinn_txn_swap_fp(FILE *oldfp, FILE *newfp) {
    JinnTxnFile *t = jinn_txn_find(oldfp);
    if (t) t->fp = newfp;
}

void jinn_txn_commit(void) {
    if (jinn_txn_depth <= 0) return;
    if (--jinn_txn_depth > 0) return;
    for (JinnTxnFile *t = jinn_txn_files; t; t = t->next) {
        if (t->fp) {
            fflush(t->fp);
            int fd = fileno(t->fp);
            if (fd >= 0 && jinn_wal_get_policy() != JINN_WAL_SYNC_NONE) {
#if defined(__linux__)
                if (fdatasync(fd) != 0)
#endif
                    (void)fsync(fd);
            }
        }
        if (t->wal && jinn_wal_get_policy() != JINN_WAL_SYNC_NONE) {
            jinn_wal_commit_group(t->wal);
        } else if (t->wal) {
            fflush(t->wal);
        }
    }
    jinn_txn_free_files();
}

void jinn_txn_rollback(void) {
    if (jinn_txn_depth <= 0) return;
    jinn_txn_depth = 0;
    for (JinnTxnFile *t = jinn_txn_files; t; t = t->next) {
        if (t->fp) {
            fseek(t->fp, 0, SEEK_SET);
            if (t->snap_len > 0) {
                if (fwrite(t->snap, 1, (size_t)t->snap_len, t->fp)
                        != (size_t)t->snap_len) {
                    fprintf(stderr, "jinn: txn: rollback restore failed\n");
                }
            }
            fflush(t->fp);
            int fd = fileno(t->fp);
            if (fd >= 0 && ftruncate(fd, (off_t)t->snap_len) != 0) {
                fprintf(stderr, "jinn: txn: rollback truncate failed\n");
            }
            fseek(t->fp, 0, SEEK_END);
        }
        if (t->wal) {
            fflush(t->wal);
            int wfd = fileno(t->wal);
            if (wfd >= 0 && ftruncate(wfd, (off_t)t->wal_off) != 0) {
                fprintf(stderr, "jinn: txn: WAL truncate failed\n");
            }
            fseek(t->wal, t->wal_off, SEEK_SET);
        }
        if (t->on_rollback) t->on_rollback(t->rb_arg);
    }
    jinn_txn_free_files();
}

/* Get WAL size (number of bytes of entries after magic). Returns 0 if empty. */
int64_t jinn_wal_size(FILE *wal) {
    if (!wal) return 0;
    long cur = ftell(wal);
    fseek(wal, 0, SEEK_END);
    long end = ftell(wal);
    fseek(wal, cur, SEEK_SET);
    return (end > 8) ? (int64_t)(end - 8) : 0;
}

/*
 * Replay WAL entries with CRC verification.
 * Calls `callback(op, payload, payload_len, timestamp, user_data)` for each
 * valid entry. Stops at first corrupted/truncated entry.
 * Returns number of entries successfully replayed, or -1 on error.
 */

int64_t jinn_wal_replay(FILE *wal, jinn_wal_replay_cb callback, void *user_data) {
    if (!wal || !callback) return -1;

    /* Save current position, seek past magic */
    long saved = ftell(wal);
    fseek(wal, 0, SEEK_END);
    long file_end = ftell(wal);
    fseek(wal, 8, SEEK_SET); /* skip magic */

    int64_t count = 0;

    while (ftell(wal) < file_end) {
        (void)ftell(wal); /* track position for diagnostics if needed */

        /* Read header: [4B payload_len][1B op][8B timestamp] */
        uint32_t payload_len;
        uint8_t  op;
        int64_t  ts;

        if (fread(&payload_len, 4, 1, wal) != 1) break;
        if (fread(&op, 1, 1, wal) != 1) break;
        if (fread(&ts, 8, 1, wal) != 1) break;

        /* Sanity check payload_len (max 64MB) */
        if (payload_len > 64 * 1024 * 1024) break;

        /* Check remaining file has enough bytes for payload + CRC */
        long remaining = file_end - ftell(wal);
        if (remaining < (long)(payload_len + 4)) break;

        /* Read payload */
        uint8_t *payload = NULL;
        if (payload_len > 0) {
            payload = (uint8_t *)malloc(payload_len);
            if (!payload) break;
            if (fread(payload, 1, payload_len, wal) != payload_len) {
                free(payload);
                break;
            }
        }

        /* Read stored CRC */
        uint32_t stored_crc;
        if (fread(&stored_crc, 4, 1, wal) != 1) {
            free(payload);
            break;
        }

        /* Verify CRC: computed over op + timestamp + payload */
        size_t crc_len = 1 + 8 + payload_len;
        uint8_t *crc_buf = (uint8_t *)malloc(crc_len);
        if (!crc_buf) {
            free(payload);
            break;
        }
        crc_buf[0] = op;
        memcpy(crc_buf + 1, &ts, 8);
        if (payload_len > 0 && payload) {
            memcpy(crc_buf + 9, payload, payload_len);
        }
        uint32_t computed_crc = crc32(crc_buf, crc_len);
        free(crc_buf);

        if (stored_crc != 0 && computed_crc != stored_crc) {
            /* CRC mismatch — entry is corrupt, stop replay */
            free(payload);
            break;
        }

        /* Valid entry — invoke callback */
        callback(op, payload, payload_len, ts, user_data);
        free(payload);
        count++;
    }

    /* Restore file position */
    fseek(wal, saved, SEEK_SET);
    return count;
}
