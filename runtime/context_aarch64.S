/*
 * aarch64 context switch: save/restore callee-saved registers.
 * jade_context_swap(jade_context_t *from, jade_context_t *to)
 *
 * jade_context_t layout:
 *   [0]    sp
 *   [8]    lr (x30)
 *   [16]   fp (x29)
 *   [24]   x19-x28 (10 × 8 = 80 bytes)
 *   [104]  d8-d15  (8 × 8 = 64 bytes)
 *   Total: 168 bytes
 */

.text
.globl jade_context_swap
.type jade_context_swap, %function
.p2align 4
jade_context_swap:
    /* Save callee-saved registers into *x0 (from) */
    mov  x2, sp
    str  x2,  [x0, #0]
    str  x30, [x0, #8]
    str  x29, [x0, #16]
    stp  x19, x20, [x0, #24]
    stp  x21, x22, [x0, #40]
    stp  x23, x24, [x0, #56]
    stp  x25, x26, [x0, #72]
    stp  x27, x28, [x0, #88]
    stp  d8,  d9,  [x0, #104]
    stp  d10, d11, [x0, #120]
    stp  d12, d13, [x0, #136]
    stp  d14, d15, [x0, #152]

    /* Restore callee-saved registers from *x1 (to) */
    ldr  x2,  [x1, #0]
    mov  sp, x2
    ldr  x30, [x1, #8]
    ldr  x29, [x1, #16]
    ldp  x19, x20, [x1, #24]
    ldp  x21, x22, [x1, #40]
    ldp  x23, x24, [x1, #56]
    ldp  x25, x26, [x1, #72]
    ldp  x27, x28, [x1, #88]
    ldp  d8,  d9,  [x1, #104]
    ldp  d10, d11, [x1, #120]
    ldp  d12, d13, [x1, #136]
    ldp  d14, d15, [x1, #152]

    ret
.size jade_context_swap, . - jade_context_swap

/* Mark the stack as non-executable */
#if defined(__linux__) && defined(__ELF__)
.section .note.GNU-stack,"",@progbits
#endif
