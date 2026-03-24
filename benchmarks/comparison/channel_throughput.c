// Channel throughput benchmark — C comparison using pipe
// Send/recv 1M i64 values through a pipe

#include <stdio.h>
#include <stdlib.h>

int main() {
    int pipefd[2];
    if (pipe(pipefd) != 0) return 1;
    long n = 1000000;
    long val;
    for (long i = 0; i < n; i++) {
        write(pipefd[1], &i, sizeof(long));
        read(pipefd[0], &val, sizeof(long));
    }
    printf("0\n");
    return 0;
}
