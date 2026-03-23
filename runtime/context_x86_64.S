/*
 * x86_64 context switch: save/restore callee-saved registers.
 * jade_context_swap(jade_context_t *from, jade_context_t *to)
 *
 * jade_context_t layout (7 × 8 = 56 bytes):
 *   [0]   rsp
 *   [8]   rbp
 *   [16]  rbx
 *   [24]  r12
 *   [32]  r13
 *   [40]  r14
 *   [48]  r15
 */

.text
.globl jade_context_swap
.type jade_context_swap, @function
.p2align 4
jade_context_swap:
    /* Save callee-saved registers into *rdi (from) */
    movq %rsp,  0(%rdi)
    movq %rbp,  8(%rdi)
    movq %rbx, 16(%rdi)
    movq %r12, 24(%rdi)
    movq %r13, 32(%rdi)
    movq %r14, 40(%rdi)
    movq %r15, 48(%rdi)

    /* Restore callee-saved registers from *rsi (to) */
    movq  0(%rsi), %rsp
    movq  8(%rsi), %rbp
    movq 16(%rsi), %rbx
    movq 24(%rsi), %r12
    movq 32(%rsi), %r13
    movq 40(%rsi), %r14
    movq 48(%rsi), %r15

    ret
.size jade_context_swap, . - jade_context_swap

/* Mark the stack as non-executable (GNU ELF note) */
#if defined(__linux__) && defined(__ELF__)
.section .note.GNU-stack,"",@progbits
#endif
