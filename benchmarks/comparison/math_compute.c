#include <stdio.h>
#include <stdint.h>

int main(void) {
    int64_t sum = 0;
    for (int64_t i = 1; i <= 10000; i++) {
        for (int64_t j = 1; j <= 10000; j++) {
            sum += i * j + i - j;
        }
    }
    printf("%ld\n", sum);
    return 0;
}
