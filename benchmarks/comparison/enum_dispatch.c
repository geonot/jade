#include <stdio.h>
#include <stdint.h>

enum OpTag { OP_ADD, OP_MUL, OP_NEG };

typedef struct {
    enum OpTag tag;
    int64_t a, b;
} Op;

static int64_t eval_op(Op op) {
    switch (op.tag) {
        case OP_ADD: return op.a + op.b;
        case OP_MUL: return op.a * op.b;
        case OP_NEG: return -op.a;
    }
    return 0;
}

int main(void) {
    int64_t total = 0;
    for (int64_t i = 0; i < 2000000000; i++) {
        total += eval_op((Op){OP_ADD, i, i + 1});
        total += eval_op((Op){OP_MUL, i, 2});
        total += eval_op((Op){OP_NEG, i, 0});
        total ^= i;
    }
    printf("%ld\n", total);
    return 0;
}
