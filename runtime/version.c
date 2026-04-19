/*
 * Jade Version File Runtime
 *
 * Append-only version log for @versioned stores.
 *
 * File format: [8B magic "JADEVER\0"][entries...]
 * Entry:       [8B sid][8B version_num][8B timestamp][rec_size bytes of record data]
 *
 * Each entry is a snapshot of the record BEFORE mutation.
 * The current (latest) record lives in the main .store file.
 * Version numbers are per-record, starting at 1 on insert.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>

static const char VER_MAGIC[8] = {'J','A','D','E','V','E','R','\0'};
#define VER_HEADER 8   /* just the magic */
#define VER_ENTRY_HDR 24  /* sid(8) + version(8) + timestamp(8) */

/* ── Open / create a versions file ──────────────────────────────── */
FILE *jade_ver_open(const char *path) {
    FILE *f = fopen(path, "r+b");
    if (f) {
        char magic[8];
        if (fread(magic, 1, 8, f) != 8 || memcmp(magic, VER_MAGIC, 8) != 0) {
            fclose(f);
            return NULL;
        }
        return f;
    }
    /* Create new */
    f = fopen(path, "w+b");
    if (!f) return NULL;
    fwrite(VER_MAGIC, 1, 8, f);
    fflush(f);
    return f;
}

/* ── Close a versions file ──────────────────────────────────────── */
void jade_ver_close(FILE *f) {
    if (f) fclose(f);
}

/* ── Append a version entry ─────────────────────────────────────── */
/* Writes the old record data before mutation. */
void jade_ver_append(FILE *f, int64_t sid, int64_t version,
                     const void *record_data, int64_t rec_size) {
    if (!f) return;
    fseek(f, 0, SEEK_END);
    int64_t ts = (int64_t)time(NULL);
    fwrite(&sid, 8, 1, f);
    fwrite(&version, 8, 1, f);
    fwrite(&ts, 8, 1, f);
    fwrite(record_data, (size_t)rec_size, 1, f);
    fflush(f);
}

/* ── Count versions for a given sid ─────────────────────────────── */
int64_t jade_ver_count(FILE *f, int64_t sid, int64_t rec_size) {
    if (!f) return 0;
    int64_t count = 0;
    (void)rec_size; /* entry_size implied by skip in fseek below */
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t entry_sid;
    while (fread(&entry_sid, 8, 1, f) == 1) {
        if (entry_sid == sid) count++;
        /* skip version(8) + timestamp(8) + record_data(rec_size) */
        fseek(f, 8 + 8 + rec_size, SEEK_CUR);
    }
    return count;
}

/* ── Retrieve a specific version of a record ────────────────────── */
/* Returns 1 if found, 0 if not. Writes record data into out_buf. */
int64_t jade_ver_at(FILE *f, int64_t sid, int64_t version,
                    void *out_buf, int64_t rec_size) {
    if (!f) return 0;
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t entry_sid, entry_ver;
    while (fread(&entry_sid, 8, 1, f) == 1) {
        fread(&entry_ver, 8, 1, f);
        /* skip timestamp */
        fseek(f, 8, SEEK_CUR);
        if (entry_sid == sid && entry_ver == version) {
            fread(out_buf, (size_t)rec_size, 1, f);
            return 1;
        }
        fseek(f, rec_size, SEEK_CUR);
    }
    return 0;
}

/* ── Retrieve all versions for a sid into a caller-allocated buffer ── */
/* Returns the number of versions written.
 * out_buf must be large enough: max_versions * rec_size bytes.
 * Versions are returned in file order (oldest first). */
int64_t jade_ver_history(FILE *f, int64_t sid,
                         void *out_buf, int64_t rec_size,
                         int64_t max_versions) {
    if (!f) return 0;
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t written = 0;
    int64_t entry_sid, entry_ver;
    uint8_t *dst = (uint8_t *)out_buf;
    while (fread(&entry_sid, 8, 1, f) == 1 && written < max_versions) {
        fread(&entry_ver, 8, 1, f);
        fseek(f, 8, SEEK_CUR); /* skip timestamp */
        if (entry_sid == sid) {
            fread(dst + written * rec_size, (size_t)rec_size, 1, f);
            written++;
        } else {
            fseek(f, rec_size, SEEK_CUR);
        }
    }
    return written;
}

/* ── Compact: keep only the latest N versions per record ────────── */
void jade_ver_compact(FILE *f, int64_t rec_size, int64_t keep_n) {
    if (!f || keep_n <= 0) return;

    /* First pass: count entries per sid */
    fseek(f, VER_HEADER, SEEK_SET);
    int64_t total = 0;
    {
        int64_t s;
        while (fread(&s, 8, 1, f) == 1) {
            total++;
            fseek(f, 8 + 8 + rec_size, SEEK_CUR);
        }
    }
    if (total == 0) return;

    /* Read all entries into memory */
    size_t entry_size = VER_ENTRY_HDR + (size_t)rec_size;
    uint8_t *entries = (uint8_t *)malloc((size_t)total * entry_size);
    if (!entries) return;

    fseek(f, VER_HEADER, SEEK_SET);
    fread(entries, entry_size, (size_t)total, f);

    /* For each unique sid, count occurrences and mark old ones for deletion.
     * Simple O(n²) — fine for compaction which is infrequent. */
    uint8_t *keep = (uint8_t *)calloc((size_t)total, 1);

    for (int64_t i = total - 1; i >= 0; i--) {
        int64_t sid_i;
        memcpy(&sid_i, entries + i * entry_size, 8);
        int64_t kept = 0;
        for (int64_t j = total - 1; j >= i; j--) {
            int64_t sid_j;
            memcpy(&sid_j, entries + j * entry_size, 8);
            if (sid_j == sid_i && keep[j]) kept++;
        }
        if (kept < keep_n) keep[i] = 1;
    }

    /* Rewrite file with only kept entries */
    fseek(f, 0, SEEK_SET);
    fwrite(VER_MAGIC, 1, 8, f);
    for (int64_t i = 0; i < total; i++) {
        if (keep[i]) {
            fwrite(entries + i * entry_size, entry_size, 1, f);
        }
    }
    /* Truncate */
    long pos = ftell(f);
    ftruncate(fileno(f), pos);
    fflush(f);

    free(entries);
    free(keep);
}
