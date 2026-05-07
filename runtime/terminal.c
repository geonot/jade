/* runtime/terminal.c — Terminal raw mode + size detection (POSIX).
 *
 * Used by std.terminal. On non-POSIX platforms, the raw-mode functions
 * return -1 so callers can fall back gracefully.
 */
#include <stdint.h>
#include "jade_rt.h"

#if defined(__unix__) || defined(__APPLE__)
#include <termios.h>
#include <unistd.h>
#include <sys/ioctl.h>

static struct termios saved_termios;
static int            saved_termios_valid = 0;

int jade_terminal_enable_raw(int fd) {
    struct termios raw;
    if (tcgetattr(fd, &saved_termios) != 0) return -1;
    saved_termios_valid = 1;
    raw = saved_termios;
    raw.c_lflag &= (tcflag_t)~(ECHO | ICANON | ISIG | IEXTEN);
    raw.c_iflag &= (tcflag_t)~(IXON | ICRNL | BRKINT | INPCK | ISTRIP);
    raw.c_oflag &= (tcflag_t)~OPOST;
    raw.c_cflag |= CS8;
    raw.c_cc[VMIN]  = 1;
    raw.c_cc[VTIME] = 0;
    return tcsetattr(fd, TCSAFLUSH, &raw);
}

int jade_terminal_disable_raw(int fd) {
    if (!saved_termios_valid) return -1;
    return tcsetattr(fd, TCSAFLUSH, &saved_termios);
}

int jade_terminal_size(int32_t *out_cols, int32_t *out_rows) {
    struct winsize ws;
    if (ioctl(STDOUT_FILENO, TIOCGWINSZ, &ws) != 0) {
        *out_cols = 80;
        *out_rows = 24;
        return -1;
    }
    *out_cols = (int32_t)ws.ws_col;
    *out_rows = (int32_t)ws.ws_row;
    return 0;
}
#else
int jade_terminal_enable_raw(int fd) { (void)fd; return -1; }
int jade_terminal_disable_raw(int fd) { (void)fd; return -1; }
int jade_terminal_size(int32_t *out_cols, int32_t *out_rows) {
    *out_cols = 80; *out_rows = 24; return -1;
}
#endif
