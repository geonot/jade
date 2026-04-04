#include <stdio.h>
#include <stdint.h>

typedef struct { int64_t x, y, z; } Vec3;

static int64_t dot(Vec3 a, Vec3 b) {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

int main(void) {
    int64_t total = 0;
    for (int64_t i = 0; i < 800000000; i++) {
        Vec3 a = {i ^ total, i + 1, i + 2};
        Vec3 b = {i + 3, i + 4, i + 5};
        total += dot(a, b);
    }
    printf("%ld\n", total);
    return 0;
}
