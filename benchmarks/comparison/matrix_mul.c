#include <stdio.h>
#include <stdint.h>

int main(void) {
    int n = 1500;
    int64_t total = 0;
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++) {
            int64_t sum = 0;
            for (int k = 0; k < n; k++)
                sum += (int64_t)(i * n + k) * (k * n + j) + (total ^ k);
            total += sum;
        }
    printf("%ld\n", total);
    return 0;
}
