#include <stdio.h>
#include <stdint.h>

static int64_t a_elem(int64_t i, int64_t j) {
    return ((i + j) * (i + j + 1)) / 2 + i + 1;
}

int main(void) {
    int64_t n = 1000, sum = 0;
    for (int iter = 0; iter < 2000; iter++)
        for (int64_t i = 0; i < n; i++) {
            int64_t acc = 0;
            for (int64_t j = 0; j < n; j++)
                acc += a_elem(i, j) * (j + 1);
            sum += acc % 1000000;
        }
    printf("%ld\n", sum);
    return 0;
}
