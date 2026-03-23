#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

/* Equivalent store benchmark in C using raw file I/O */

#define HEADER_SIZE 24
#define MAGIC "JADESTR\0"

typedef struct {
    int64_t key;
    int64_t value;
} Record;

static FILE *fp = NULL;
static const char *FILENAME = "records_c.store";

static void ensure_open(void) {
    if (fp) return;
    fp = fopen(FILENAME, "r+b");
    if (!fp) {
        fp = fopen(FILENAME, "w+b");
        fwrite(MAGIC, 1, 8, fp);
        int64_t zero = 0;
        fwrite(&zero, 8, 1, fp);  /* count */
        int64_t rsz = sizeof(Record);
        fwrite(&rsz, 8, 1, fp);  /* rec_size */
        fflush(fp);
    }
}

static void insert_record(int64_t key, int64_t value) {
    ensure_open();
    /* Read count */
    fseek(fp, 8, SEEK_SET);
    int64_t count = 0;
    fread(&count, 8, 1, fp);
    /* Write record at end */
    fseek(fp, HEADER_SIZE + count * sizeof(Record), SEEK_SET);
    Record r = {key, value};
    fwrite(&r, sizeof(Record), 1, fp);
    /* Update count */
    count++;
    fseek(fp, 8, SEEK_SET);
    fwrite(&count, 8, 1, fp);
    fflush(fp);
}

static Record query_by_key(int64_t key) {
    ensure_open();
    fseek(fp, 8, SEEK_SET);
    int64_t count = 0;
    fread(&count, 8, 1, fp);
    fseek(fp, HEADER_SIZE, SEEK_SET);
    Record r;
    for (int64_t i = 0; i < count; i++) {
        fread(&r, sizeof(Record), 1, fp);
        if (r.key == key) return r;
    }
    memset(&r, 0, sizeof(r));
    return r;
}

static int64_t count_records(void) {
    ensure_open();
    fseek(fp, 8, SEEK_SET);
    int64_t count = 0;
    fread(&count, 8, 1, fp);
    return count;
}

int main(void) {
    remove(FILENAME);
    /* Insert 10000 records */
    for (int64_t i = 0; i < 10000; i++) {
        insert_record(i, i * 7);
    }
    /* Query 1000 times */
    int64_t total = 0;
    for (int64_t j = 0; j < 1000; j++) {
        Record r = query_by_key(j);
        total += r.value;
    }
    printf("%ld\n", total);
    printf("%ld\n", count_records());
    if (fp) fclose(fp);
    remove(FILENAME);
    return 0;
}
