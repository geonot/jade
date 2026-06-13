/*
 * runtime/migrate.c — Schema migration engine for Jinn stores
 *
 * Provides:
 *   - jinn_mig_add_field:  rewrite store, inserting a new field into every record
 *   - jinn_mig_drop_field: rewrite store, removing a field from every record
 *   - jinn_mig_log_open:   open/create the migrations.log file
 *   - jinn_mig_log_close:  close the log
 *   - jinn_mig_log_applied: check if a version was already applied
 *   - jinn_mig_log_record:  record a newly applied migration
 *
 * Store file format (header = 24 bytes):
 *   [8B magic "JINNSTR\0"][8B count][8B rec_size][records...]
 *
 * Migration log format (header = 8 bytes):
 *   [8B magic "JINNMIG\0"][entries...]
 *   Entry: [8B version][8B timestamp][1B direction (1=up, 0=down)]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdint.h>
#include "jinn_rt.h"

#define STORE_HEADER 40
#define STORE_MAGIC  "JADESTR\0"
#define STORE_FP_OFFSET  24
#define STORE_VER_OFFSET 32
#define MIG_HEADER   8
#define MIG_MAGIC    "JINNMIG\0"
#define MIG_ENTRY    17   /* 8 + 8 + 1 */

/*
 * Schema fingerprint guard.
 *
 * On opening an existing store, the compiler emits a call comparing the
 * compile-time schema fingerprint against the value persisted in the
 * header (bytes 24..32).  A mismatch means the on-disk layout no longer
 * matches the program's declared schema.
 *
 * Behaviour:
 *   - stored == 0           legacy/unstamped file: stamp it and proceed.
 *   - stored == expected    schema matches: proceed.
 *   - otherwise             abort with a precise diagnostic instructing
 *                           the programmer to add a migration.
 */
static int jinn_migration_active = 0;

void jinn_migration_enter(void) { jinn_migration_active++; }
void jinn_migration_leave(void) {
    if (jinn_migration_active > 0) jinn_migration_active--;
}

void jinn_store_check_schema(FILE *fp, int64_t expected_fp,
                             int64_t expected_ver, const char *store_name) {
    if (!fp) return;
    if (jinn_migration_active) return;
    long saved = ftell(fp);
    int64_t stored_fp = 0, stored_ver = 0;
    if (fseek(fp, STORE_FP_OFFSET, SEEK_SET) == 0) {
        if (fread(&stored_fp, 8, 1, fp) != 1) stored_fp = 0;
        if (fread(&stored_ver, 8, 1, fp) != 1) stored_ver = 0;
    }
    if (stored_fp == expected_fp) {
        if (saved >= 0) fseek(fp, saved, SEEK_SET);
        return;
    }
    if (stored_fp == 0) {
        /* unstamped legacy file: stamp and proceed */
        fseek(fp, STORE_FP_OFFSET, SEEK_SET);
        fwrite(&expected_fp, 8, 1, fp);
        fwrite(&expected_ver, 8, 1, fp);
        fflush(fp);
        if (saved >= 0) fseek(fp, saved, SEEK_SET);
        return;
    }
    fprintf(stderr,
            "jinn: store '%s' schema mismatch: on-disk fingerprint %lld "
            "(version %lld) does not match the program's schema "
            "(fingerprint %lld, version %lld).\n"
            "      The store layout changed without a bridging migration. "
            "Add a `migration` that advances the schema, or delete %s.store "
            "to recreate it.\n",
            store_name ? store_name : "?",
            (long long)stored_fp, (long long)stored_ver,
            (long long)expected_fp, (long long)expected_ver,
            store_name ? store_name : "?");
    fflush(stderr);
    abort();
}

/*
 * Stamp the schema fingerprint/version into a store header.  Called by
 * generated migration code after rewriting records to the new layout so
 * that the next open sees a matching fingerprint.
 */
void jinn_store_stamp_schema(FILE **store_fp_ptr, int64_t fingerprint,
                             int64_t version) {
    if (!store_fp_ptr || !*store_fp_ptr) return;
    FILE *fp = *store_fp_ptr;
    long saved = ftell(fp);
    fseek(fp, STORE_FP_OFFSET, SEEK_SET);
    fwrite(&fingerprint, 8, 1, fp);
    fwrite(&version, 8, 1, fp);
    fflush(fp);
    if (saved >= 0) fseek(fp, saved, SEEK_SET);
}

/* ─── Migration log ─────────────────────────────────────────────── */

FILE *jinn_mig_log_open(const char *path) {
    FILE *fp = fopen(path, "r+b");
    if (fp) return fp;
    /* create new */
    fp = fopen(path, "w+b");
    if (!fp) return NULL;
    fwrite(MIG_MAGIC, 1, MIG_HEADER, fp);
    fflush(fp);
    return fp;
}

void jinn_mig_log_close(FILE *fp) {
    if (fp) fclose(fp);
}

/*
 * Check if a particular migration version has been applied (direction=up).
 * Scans the log in reverse so that the latest entry for a version wins.
 * Returns 1 if applied, 0 if not.
 */
int64_t jinn_mig_log_applied(FILE *fp, int64_t version) {
    if (!fp) return 0;
    fseek(fp, 0, SEEK_END);
    long end = ftell(fp);
    long pos = MIG_HEADER;
    int64_t result = 0;
    /* scan all entries, last one for this version wins */
    while (pos + MIG_ENTRY <= end) {
        fseek(fp, pos, SEEK_SET);
        int64_t v;
        fread(&v, 8, 1, fp);
        int64_t ts;
        fread(&ts, 8, 1, fp);
        uint8_t dir;
        fread(&dir, 1, 1, fp);
        if (v == version) {
            result = (dir == 1) ? 1 : 0;
        }
        pos += MIG_ENTRY;
    }
    return result;
}

/*
 * Record that a migration was applied.
 * direction: 1 = up, 0 = down
 */
void jinn_mig_log_record(FILE *fp, int64_t version, int64_t direction) {
    if (!fp) return;
    fseek(fp, 0, SEEK_END);
    fwrite(&version, 8, 1, fp);
    int64_t ts = (int64_t)time(NULL);
    fwrite(&ts, 8, 1, fp);
    uint8_t dir = (uint8_t)(direction & 0xFF);
    fwrite(&dir, 1, 1, fp);
    fflush(fp);
}

/* ─── Store rewriting ───────────────────────────────────────────── */

/*
 * Rewrite a store file, inserting `field_size` bytes at `field_offset`
 * in every record.  The inserted bytes are copied from `default_val`
 * (which must be at least `field_size` bytes long, or NULL for zeros).
 *
 * Parameters:
 *   store_fp_ptr  — pointer to the FILE* global (will be updated after rewrite)
 *   store_path    — path to the .store file (for reopen)
 *   field_offset  — byte offset within the OLD record where new field goes
 *   field_size    — size of the new field in bytes
 *   default_val   — pointer to default value bytes (or NULL for zero-fill)
 *
 * Returns 0 on success, -1 on error.
 */
int64_t jinn_mig_add_field(FILE **store_fp_ptr, const char *store_path,
                           int64_t field_offset, int64_t field_size,
                           const void *default_val) {
    FILE *fp = *store_fp_ptr;
    if (!fp) return -1;

    /* read header */
    fseek(fp, 8, SEEK_SET);
    int64_t count, old_rec_size;
    fread(&count, 8, 1, fp);
    fread(&old_rec_size, 8, 1, fp);
    if (count < 0 || old_rec_size <= 0 || field_offset < 0 || field_size <= 0) return -1;
    if (count > 0 && old_rec_size > (INT64_MAX / count)) return -1;
    int64_t new_rec_size = old_rec_size + field_size;
    if (new_rec_size <= 0) return -1;

    if (count == 0) {
        /* no records — just update rec_size in header */
        fseek(fp, 16, SEEK_SET);
        fwrite(&new_rec_size, 8, 1, fp);
        fflush(fp);
        return 0;
    }

    /* read all records */
    uint8_t *old_data = (uint8_t *)malloc((size_t)(count * old_rec_size));
    if (!old_data) return -1;
    fseek(fp, STORE_HEADER, SEEK_SET);
    fread(old_data, (size_t)old_rec_size, (size_t)count, fp);

    /* build new records */
    uint8_t *new_data = (uint8_t *)calloc((size_t)count, (size_t)new_rec_size);
    if (!new_data) { free(old_data); return -1; }

    for (int64_t i = 0; i < count; i++) {
        uint8_t *src = old_data + i * old_rec_size;
        uint8_t *dst = new_data + i * new_rec_size;
        /* copy bytes before the new field */
        if (field_offset > 0)
            memcpy(dst, src, (size_t)field_offset);
        /* insert default value (or zeros — calloc already zeroed) */
        if (default_val)
            memcpy(dst + field_offset, default_val, (size_t)field_size);
        /* copy bytes after the new field */
        int64_t tail = old_rec_size - field_offset;
        if (tail > 0)
            memcpy(dst + field_offset + field_size,
                   src + field_offset, (size_t)tail);
    }

    /* close, rewrite, reopen */
    fclose(fp);
    fp = fopen(store_path, "w+b");
    if (!fp) { free(old_data); free(new_data); return -1; }

    /* write header (magic, count, rec_size, fingerprint, version) */
    fwrite(STORE_MAGIC, 1, 8, fp);
    fwrite(&count, 8, 1, fp);
    fwrite(&new_rec_size, 8, 1, fp);
    int64_t zero = 0;
    fwrite(&zero, 8, 1, fp);
    fwrite(&zero, 8, 1, fp);
    /* write records */
    fwrite(new_data, (size_t)new_rec_size, (size_t)count, fp);
    fflush(fp);

    free(old_data);
    free(new_data);
    *store_fp_ptr = fp;
    return 0;
}

/*
 * Rewrite a store file, removing `field_size` bytes at `field_offset`
 * from every record.
 *
 * Returns 0 on success, -1 on error.
 */
int64_t jinn_mig_drop_field(FILE **store_fp_ptr, const char *store_path,
                            int64_t field_offset, int64_t field_size) {
    FILE *fp = *store_fp_ptr;
    if (!fp) return -1;

    /* read header */
    fseek(fp, 8, SEEK_SET);
    int64_t count, old_rec_size;
    fread(&count, 8, 1, fp);
    fread(&old_rec_size, 8, 1, fp);
    if (count < 0 || old_rec_size <= 0 || field_offset < 0 || field_size <= 0) return -1;
    if (count > 0 && old_rec_size > (INT64_MAX / count)) return -1;
    int64_t new_rec_size = old_rec_size - field_size;

    if (new_rec_size <= 0) return -1;

    if (count == 0) {
        fseek(fp, 16, SEEK_SET);
        fwrite(&new_rec_size, 8, 1, fp);
        fflush(fp);
        return 0;
    }

    /* read all records */
    uint8_t *old_data = (uint8_t *)malloc((size_t)(count * old_rec_size));
    if (!old_data) return -1;
    fseek(fp, STORE_HEADER, SEEK_SET);
    fread(old_data, (size_t)old_rec_size, (size_t)count, fp);

    /* build new records */
    uint8_t *new_data = (uint8_t *)calloc((size_t)count, (size_t)new_rec_size);
    if (!new_data) { free(old_data); return -1; }

    for (int64_t i = 0; i < count; i++) {
        uint8_t *src = old_data + i * old_rec_size;
        uint8_t *dst = new_data + i * new_rec_size;
        /* copy bytes before the dropped field */
        if (field_offset > 0)
            memcpy(dst, src, (size_t)field_offset);
        /* copy bytes after the dropped field */
        int64_t tail = old_rec_size - field_offset - field_size;
        if (tail > 0)
            memcpy(dst + field_offset,
                   src + field_offset + field_size, (size_t)tail);
    }

    /* close, rewrite, reopen */
    fclose(fp);
    fp = fopen(store_path, "w+b");
    if (!fp) { free(old_data); free(new_data); return -1; }

    fwrite(STORE_MAGIC, 1, 8, fp);
    fwrite(&count, 8, 1, fp);
    fwrite(&new_rec_size, 8, 1, fp);
    int64_t zero2 = 0;
    fwrite(&zero2, 8, 1, fp);
    fwrite(&zero2, 8, 1, fp);
    fwrite(new_data, (size_t)new_rec_size, (size_t)count, fp);
    fflush(fp);

    free(old_data);
    free(new_data);
    *store_fp_ptr = fp;
    return 0;
}

/*
 * Compaction / vacuum.
 *
 * Rewrites a store file dropping every record whose `deleted` field (an
 * int64 tombstone timestamp at byte `deleted_offset`) is non-zero.  The
 * schema fingerprint and version are preserved — compaction never changes
 * the layout.  Returns the number of records reclaimed, or -1 on error.
 */
int64_t jinn_store_compact(FILE **store_fp_ptr, const char *store_path,
                           int64_t deleted_offset) {
    FILE *fp = *store_fp_ptr;
    if (!fp || deleted_offset < 0) return -1;

    fseek(fp, 8, SEEK_SET);
    int64_t count = 0, rec_size = 0;
    if (fread(&count, 8, 1, fp) != 1) return -1;
    if (fread(&rec_size, 8, 1, fp) != 1) return -1;
    int64_t stored_fp = 0, stored_ver = 0;
    if (fread(&stored_fp, 8, 1, fp) != 1) stored_fp = 0;
    if (fread(&stored_ver, 8, 1, fp) != 1) stored_ver = 0;
    if (count < 0 || rec_size <= 0) return -1;
    if (deleted_offset + 8 > rec_size) return -1;
    if (count == 0) return 0;
    if (rec_size > (INT64_MAX / count)) return -1;

    uint8_t *data = (uint8_t *)malloc((size_t)(count * rec_size));
    if (!data) return -1;
    fseek(fp, STORE_HEADER, SEEK_SET);
    if (fread(data, (size_t)rec_size, (size_t)count, fp) != (size_t)count) {
        free(data);
        return -1;
    }

    int64_t kept = 0;
    for (int64_t i = 0; i < count; i++) {
        uint8_t *src = data + i * rec_size;
        int64_t tomb = 0;
        memcpy(&tomb, src + deleted_offset, 8);
        if (tomb != 0) continue;
        if (kept != i) memcpy(data + kept * rec_size, src, (size_t)rec_size);
        kept++;
    }

    int64_t reclaimed = count - kept;
    if (reclaimed == 0) {
        free(data);
        return 0;
    }

    fclose(fp);
    fp = fopen(store_path, "w+b");
    if (!fp) { free(data); return -1; }

    fwrite(STORE_MAGIC, 1, 8, fp);
    fwrite(&kept, 8, 1, fp);
    fwrite(&rec_size, 8, 1, fp);
    fwrite(&stored_fp, 8, 1, fp);
    fwrite(&stored_ver, 8, 1, fp);
    if (kept > 0) fwrite(data, (size_t)rec_size, (size_t)kept, fp);
    fflush(fp);

    free(data);
    *store_fp_ptr = fp;
    return reclaimed;
}

/*
 * Auto-policy compaction.  Counts live tombstones; if the count is at or
 * above `threshold` (and threshold > 0), performs a full compaction.
 * Returns records reclaimed, 0 if below threshold, -1 on error.
 */
int64_t jinn_store_compact_if(FILE **store_fp_ptr, const char *store_path,
                              int64_t deleted_offset, int64_t threshold) {
    FILE *fp = *store_fp_ptr;
    if (!fp || deleted_offset < 0 || threshold <= 0) return 0;

    fseek(fp, 8, SEEK_SET);
    int64_t count = 0, rec_size = 0;
    if (fread(&count, 8, 1, fp) != 1) return -1;
    if (fread(&rec_size, 8, 1, fp) != 1) return -1;
    if (count <= 0 || rec_size <= 0) return 0;
    if (deleted_offset + 8 > rec_size) return -1;
    if (rec_size > (INT64_MAX / count)) return -1;

    int64_t tombs = 0;
    uint8_t *cell = (uint8_t *)malloc(8);
    if (!cell) return -1;
    for (int64_t i = 0; i < count; i++) {
        if (fseek(fp, STORE_HEADER + i * rec_size + deleted_offset, SEEK_SET) != 0) {
            free(cell);
            return -1;
        }
        int64_t tomb = 0;
        if (fread(&tomb, 8, 1, fp) != 1) { free(cell); return -1; }
        if (tomb != 0) tombs++;
    }
    free(cell);

    if (tombs < threshold) return 0;
    return jinn_store_compact(store_fp_ptr, store_path, deleted_offset);
}
