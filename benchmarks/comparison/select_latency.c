// Select latency benchmark — C comparison using poll on pipes
#include <stdio.h>
#include <poll.h>
#include <unistd.h>

int main() {
    int p1[2], p2[2];
    pipe(p1);
    pipe(p2);
    long n = 100000;
    long total = 0;
    struct pollfd fds[2];
    fds[0].fd = p1[0]; fds[0].events = POLLIN;
    fds[1].fd = p2[0]; fds[1].events = POLLIN;
    for (long i = 0; i < n; i++) {
        write(p1[1], &i, sizeof(long));
        poll(fds, 2, -1);
        if (fds[0].revents & POLLIN) {
            long val;
            read(p1[0], &val, sizeof(long));
            total += val;
        }
    }
    printf("%ld\n", total);
    return 0;
}
