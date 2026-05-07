/*
 * runtime/migrate.c — Schema migration engine for Jade stores
 *
 * Provides:
 *   - jade_mig_add_field:  rewrite store, inserting a new field into every record
 *   - jade_mig_drop_field: rewrite store, removing a field from every record
 *   - jade_mig_log_open:   open/create the migrations.log file
 *   - jade_mig_log_close:  close the log
 *   - jade_mig_log_applied: check if a version was already applied
 *   - jade_mig_log_record:  record a newly applied migration
 *
 * Store file format (header = 24 bytes):
 *   [8B magic "JADESTR\0"][8B count][8B rec_size][records...]
 *
 * Migration log format (header = 8 bytes):
 *   [8B magic "JADEMIG\0"][entries...]
 *   Entry: [8B version][8B timestamp][1B direction (1=up, 0=down)]
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdint.h>
#include "jade_rt.h"

#define STORE_HEADER 24
#define STORE_MAGIC  "JADESTR\0"
#define MIG_HEADER   8
#define MIG_MAGIC    "JADEMIG\0"
#define MIG_ENTRY    17   /* 8 + 8 + 1 */

/* ─── Migration log ─────────────────────────────────────────────── */

FILE *jade_mig_log_open(const char *path) {
    FILE *fp = fopen(path, "r+b");
    if (fp) return fp;
    /* create new */
    fp = fopen(path, "w+b");
    if (!fp) return NULL;
    fwrite(MIG_MAGIC, 1, MIG_HEADER, fp);
    fflush(fp);
    return fp;
}

void jade_mig_log_close(FILE *fp) {
    if (fp) fclose(fp);
}

/*
 * Check if a particular migration version has been applied (direction=up).
 * Scans the log in reverse so that the latest entry for a version wins.
 * Returns 1 if applied, 0 if not.
 */
int64_t jade_mig_log_applied(FILE *fp, int64_t version) {
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
void jade_mig_log_record(FILE *fp, int64_t version, int64_t direction) {
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
int64_t jade_mig_add_field(FILE **store_fp_ptr, const char *store_path,
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

    /* write header */
    fwrite(STORE_MAGIC, 1, 8, fp);
    fwrite(&count, 8, 1, fp);
    fwrite(&new_rec_size, 8, 1, fp);
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
int64_t jade_mig_drop_field(FILE **store_fp_ptr, const char *store_path,
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
    fwrite(new_data, (size_t)new_rec_size, (size_t)count, fp);
    fflush(fp);

    free(old_data);
    free(new_data);
    *store_fp_ptr = fp;
    return 0;
}
