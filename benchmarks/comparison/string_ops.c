#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>

int main() {
    int64_t total = 0;
    for (int64_t i = 0; i < 8000000; i++) {
        char *s;
        int len = asprintf(&s, "item_%ld_value", i);
        total += len;
        total ^= i;
        free(s);
    }
    printf("%ld\n", total);
    return 0;
}
