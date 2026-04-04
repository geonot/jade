#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

/* Equivalent store benchmark in C using in-memory array (matches Jade store semantics) */

typedef struct {
    int64_t key;
    int64_t value;
} Record;

static Record *records = NULL;
static int64_t count = 0;
static int64_t capacity = 0;

static void insert_record(int64_t key, int64_t value) {
    if (count >= capacity) {
        capacity = capacity ? capacity * 2 : 1024;
        records = realloc(records, (size_t)capacity * sizeof(Record));
    }
    records[count++] = (Record){key, value};
}

static Record query_by_key(int64_t key) {
    for (int64_t i = 0; i < count; i++) {
        if (records[i].key == key) return records[i];
    }
    return (Record){0, 0};
}

static int64_t count_records(void) {
    return count;
}

int main(void) {
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
    free(records);
    return 0;
}
