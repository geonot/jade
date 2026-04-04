#include <stdio.h>
#include <stdint.h>

int main(void) {
    int64_t total = 0;
    for (int64_t i = 0; i < 1500000000; i++) {
        int64_t arr[5] = {i ^ total, i + 1, i + 2, i + 3, i + 4};
        total += arr[0] + arr[1] + arr[2] + arr[3] + arr[4];
    }
    printf("%ld\n", total);
    return 0;
}
