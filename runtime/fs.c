/* runtime/fs.c — Wrappers for filesystem functions that collide with Jade names */
#include <sys/stat.h>
#include <unistd.h>
#include <stdio.h>
#include <dirent.h>
#include <string.h>
#include <math.h>

#include "jade_rt.h"
int c_mkdir(const char *path, int mode) { return mkdir(path, (mode_t)mode); }
int c_rmdir(const char *path) { return rmdir(path); }
int c_remove(const char *path) { return remove(path); }
int c_rename(const char *old, const char *new_name) { return rename(old, new_name); }
int c_chdir(const char *path) { return chdir(path); }
int c_symlink(const char *target, const char *linkpath) { return symlink(target, linkpath); }

/* Portable dirent helpers — avoids hardcoded struct offsets */
const char *jade_dirent_name(void *ent) {
    return ((struct dirent *)ent)->d_name;
}

/* Stat-based checks */
int jade_is_dir(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    return S_ISDIR(st.st_mode);
}

int jade_is_file(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    return S_ISREG(st.st_mode);
}

int jade_is_symlink(const char *path) {
    struct stat st;
    if (lstat(path, &st) != 0) return 0;
    return S_ISLNK(st.st_mode);
}

long jade_file_mtime(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return -1;
    return (long)st.st_mtime;
}

long jade_file_size(const char *path) {
    struct stat st;
    if (stat(path, &st) != 0) return -1;
    return (long)st.st_size;
}

/* Get file size from open file descriptor (for mmap) */
long fstat_size(int fd) {
    struct stat st;
    if (fstat(fd, &st) != 0) return -1;
    return (long)st.st_size;
}

/* Wrapper for close() since "close" is a Jade keyword */
int jade_fd_close(int fd) {
    return close(fd);
}

int jade_chmod(const char *path, int mode) {
    return chmod(path, (mode_t)mode);
}

/* Math wrappers for functions whose names collide with Jade */
double c_hypot(double x, double y) { return hypot(x, y); }

/* OS helpers — avoid declaring malloc/free/strlen in Jade modules */
#include <stdlib.h>
const char *jade_hostname(void) {
    static char buf[256];
    if (gethostname(buf, sizeof(buf)) == 0) return buf;
    return "";
}

const char *jade_cwd(void) {
    static char buf[4096];
    if (getcwd(buf, sizeof(buf)) != NULL) return buf;
    return "";
}
