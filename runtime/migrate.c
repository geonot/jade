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
#define MIG_ENTRY    17
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
    if (stored_fp == -1) {
        fprintf(stderr,
                "jinn: store '%s': a schema migration was interrupted before it "
                "finished — the store's on-disk layout may not match any schema "
                "version.\n"
                "      Restore %s.store from a backup, or delete it to recreate "
                "the store.\n",
                store_name ? store_name : "?", store_name ? store_name : "?");
        fflush(stderr);
        abort();
    }
    if (stored_fp == 0) {
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
FILE *jinn_mig_log_open(const char *path) {
    FILE *fp = fopen(path, "r+b");
    if (fp) return fp;
    fp = fopen(path, "w+b");
    if (!fp) return NULL;
    fwrite(MIG_MAGIC, 1, MIG_HEADER, fp);
    fflush(fp);
    return fp;
}
void jinn_mig_log_close(FILE *fp) {
    if (fp) fclose(fp);
}

int64_t jinn_mig_log_applied(FILE *fp, int64_t version) {
    if (!fp) return 0;
    fseek(fp, 0, SEEK_END);
    long end = ftell(fp);
    long pos = MIG_HEADER;
    int64_t result = 0;
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
typedef struct {
    int64_t        count;
    int64_t        rec_size;
    int64_t        fingerprint;
    int64_t        version;
    const uint8_t *records;
} MigImage;
static int mig_fill(FILE *tmp, void *arg) {
    MigImage *im = (MigImage *)arg;
    if (fwrite(STORE_MAGIC, 1, 8, tmp) != 8 ||
        fwrite(&im->count, 8, 1, tmp) != 1 ||
        fwrite(&im->rec_size, 8, 1, tmp) != 1 ||
        fwrite(&im->fingerprint, 8, 1, tmp) != 1 ||
        fwrite(&im->version, 8, 1, tmp) != 1) {
        return -1;
    }
    if (im->count > 0 &&
        fwrite(im->records, (size_t)im->rec_size, (size_t)im->count, tmp)
            != (size_t)im->count) {
        return -1;
    }
    return 0;
}
int64_t jinn_mig_add_field(FILE **store_fp_ptr, const char *store_path,
                           int64_t field_offset, int64_t field_size,
                           const void *default_val) {
    FILE *fp = *store_fp_ptr;
    if (!fp) return -1;
    fseek(fp, 8, SEEK_SET);
    int64_t count = -1, old_rec_size = 0;
    if (fread(&count, 8, 1, fp) != 1 || fread(&old_rec_size, 8, 1, fp) != 1) {
        return -1;
    }
    if (count < 0 || old_rec_size <= 0 || field_offset < 0 || field_size <= 0) return -1;
    if (count > 0 && old_rec_size > (INT64_MAX / count)) return -1;
    int64_t new_rec_size = old_rec_size + field_size;
    if (new_rec_size <= 0) return -1;
    if (count == 0) {
        fseek(fp, 16, SEEK_SET);
        fwrite(&new_rec_size, 8, 1, fp);
        int64_t in_progress = -1;
        fseek(fp, STORE_FP_OFFSET, SEEK_SET);
        fwrite(&in_progress, 8, 1, fp);
        fflush(fp);
        return 0;
    }
    uint8_t *old_data = (uint8_t *)malloc((size_t)(count * old_rec_size));
    if (!old_data) return -1;
    fseek(fp, STORE_HEADER, SEEK_SET);
    if (fread(old_data, (size_t)old_rec_size, (size_t)count, fp) != (size_t)count) {
        fprintf(stderr, "jinn: migrate: short read of %s — aborting migration\n",
                store_path);
        free(old_data);
        return -1;
    }
    uint8_t *new_data = (uint8_t *)calloc((size_t)count, (size_t)new_rec_size);
    if (!new_data) { free(old_data); return -1; }
    for (int64_t i = 0; i < count; i++) {
        uint8_t *src = old_data + i * old_rec_size;
        uint8_t *dst = new_data + i * new_rec_size;
        if (field_offset > 0)
            memcpy(dst, src, (size_t)field_offset);
        if (default_val)
            memcpy(dst + field_offset, default_val, (size_t)field_size);
        int64_t tail = old_rec_size - field_offset;
        if (tail > 0)
            memcpy(dst + field_offset + field_size,
                   src + field_offset, (size_t)tail);
    }
    MigImage im = { count, new_rec_size, -1, 0, new_data };
    if (jinn_atomic_rewrite_reopen(store_path, mig_fill, &im, store_fp_ptr) != 0) {
        free(old_data);
        free(new_data);
        return -1;
    }
    free(old_data);
    free(new_data);
    return 0;
}
int64_t jinn_mig_drop_field(FILE **store_fp_ptr, const char *store_path,
                            int64_t field_offset, int64_t field_size) {
    FILE *fp = *store_fp_ptr;
    if (!fp) return -1;
    fseek(fp, 8, SEEK_SET);
    int64_t count = -1, old_rec_size = 0;
    if (fread(&count, 8, 1, fp) != 1 || fread(&old_rec_size, 8, 1, fp) != 1) {
        return -1;
    }
    if (count < 0 || old_rec_size <= 0 || field_offset < 0 || field_size <= 0) return -1;
    if (count > 0 && old_rec_size > (INT64_MAX / count)) return -1;
    int64_t new_rec_size = old_rec_size - field_size;
    if (new_rec_size <= 0) return -1;

    if (count == 0) {
        fseek(fp, 16, SEEK_SET);
        fwrite(&new_rec_size, 8, 1, fp);
        int64_t in_progress = -1;
        fseek(fp, STORE_FP_OFFSET, SEEK_SET);
        fwrite(&in_progress, 8, 1, fp);
        fflush(fp);
        return 0;
    }
    uint8_t *old_data = (uint8_t *)malloc((size_t)(count * old_rec_size));
    if (!old_data) return -1;
    fseek(fp, STORE_HEADER, SEEK_SET);
    if (fread(old_data, (size_t)old_rec_size, (size_t)count, fp) != (size_t)count) {
        fprintf(stderr, "jinn: migrate: short read of %s — aborting migration\n",
                store_path);
        free(old_data);
        return -1;
    }
    uint8_t *new_data = (uint8_t *)calloc((size_t)count, (size_t)new_rec_size);
    if (!new_data) { free(old_data); return -1; }
    for (int64_t i = 0; i < count; i++) {
        uint8_t *src = old_data + i * old_rec_size;
        uint8_t *dst = new_data + i * new_rec_size;
        if (field_offset > 0)
            memcpy(dst, src, (size_t)field_offset);
        int64_t tail = old_rec_size - field_offset - field_size;
        if (tail > 0)
            memcpy(dst + field_offset,
                   src + field_offset + field_size, (size_t)tail);
    }
    MigImage im = { count, new_rec_size, -1, 0, new_data };
    if (jinn_atomic_rewrite_reopen(store_path, mig_fill, &im, store_fp_ptr) != 0) {
        free(old_data);
        free(new_data);
        return -1;
    }
    free(old_data);
    free(new_data);
    return 0;
}
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
    MigImage im = { kept, rec_size, stored_fp, stored_ver, data };
    if (jinn_atomic_rewrite_reopen(store_path, mig_fill, &im, store_fp_ptr) != 0) {
        free(data);
        return -1;
    }

    free(data);
    return reclaimed;
}
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
