#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>
#include "jinn_rt.h"

static pthread_mutex_t journal_mu = PTHREAD_MUTEX_INITIALIZER;
static int journal_ready;
static int journal_lock_fd = -1;
static FILE *journal;
static char journal_directory[PATH_MAX];
static dev_t journal_device;
static ino_t journal_inode;
static const char journal_path[] = ".jinn-transaction";
static const unsigned char journal_magic[8] = {'J','I','N','N','T','X','N','1'};

void jinn_persistence_fail(const char *phase) {
    fprintf(stderr, "jinn: persistence: %s failed; outcome may be indeterminate; aborting with recovery evidence preserved\n", phase);
    abort();
}

void jinn_persistence_sync(FILE *fp) {
    if (!fp || fflush(fp) != 0 || fileno(fp) < 0 || fsync(fileno(fp)) != 0)
        jinn_persistence_fail("file synchronization");
}

size_t jinn_store_fread(void *data, size_t size, size_t count, FILE *fp) {
    if (!fp || (size && count > SIZE_MAX / size)) jinn_persistence_fail("store read arguments");
    size_t got = fread(data, size, count, fp);
    if (got != count || ferror(fp)) jinn_persistence_fail("store read");
    return got;
}

size_t jinn_store_fwrite(const void *data, size_t size, size_t count, FILE *fp) {
    if (!fp || (size && count > SIZE_MAX / size)) jinn_persistence_fail("store write arguments");
    size_t got = fwrite(data, size, count, fp);
    if (got != count || ferror(fp)) jinn_persistence_fail("store write");
    return got;
}

int jinn_store_fseek(FILE *fp, long offset, int whence) {
    if (!fp || fseek(fp, offset, whence) != 0) jinn_persistence_fail("store seek");
    return 0;
}

int jinn_store_fflush(FILE *fp) {
    if (!fp || fflush(fp) != 0) jinn_persistence_fail("store flush");
    return 0;
}

static uint64_t journal_hash(uint64_t h, const void *data, size_t len) {
    const unsigned char *p = data;
    for (size_t i = 0; i < len; i++) h = (h ^ p[i]) * UINT64_C(1099511628211);
    return h;
}

typedef struct JournalImage {
    char *path;
    unsigned char *data;
    uint64_t len;
    struct JournalImage *next;
} JournalImage;

static int journal_fill(FILE *fp, void *arg) {
    JournalImage *im = arg;
    return im->len == 0 || fwrite(im->data, 1, (size_t)im->len, fp) == im->len ? 0 : -1;
}

static void journal_remove(void) {
    if (unlink(journal_path) != 0 || jinn_dir_fsync(journal_path) != 0)
        jinn_persistence_fail("transaction journal retirement");
}

static void persistence_check_path(const char *path);

static void journal_recover(void) {
    FILE *fp = fopen(journal_path, "rb");
    if (!fp) {
        if (errno != ENOENT) jinn_persistence_fail("transaction journal open");
        return;
    }
    struct stat st;
    if (fstat(fileno(fp), &st) != 0) jinn_persistence_fail("transaction journal size");
    if (st.st_size < 8) {
        if (fclose(fp) != 0) jinn_persistence_fail("transaction journal close");
        journal_remove();
        return;
    }
    unsigned char magic[8];
    if (fread(magic, 1, 8, fp) != 8 || memcmp(magic, journal_magic, 8) != 0)
        jinn_persistence_fail("transaction journal header validation");
    JournalImage *images = NULL;
    for (;;) {
        uint32_t path_len;
        uint64_t len, expected;
        size_t got = fread(&path_len, 1, sizeof path_len, fp);
        if (got == 0 && feof(fp)) break;
        if (got != sizeof path_len || fread(&len, sizeof len, 1, fp) != 1) {
            if (ferror(fp)) jinn_persistence_fail("transaction journal read");
            break;
        }
        if (path_len == 0 || path_len >= PATH_MAX || len > SIZE_MAX || len > INT64_MAX)
            jinn_persistence_fail("transaction journal lengths");
        long pos = ftell(fp);
        if (pos < 0 || pos > st.st_size) jinn_persistence_fail("transaction journal position");
        uint64_t remaining = (uint64_t)(st.st_size - pos);
        if ((uint64_t)path_len + sizeof expected > remaining ||
            len > remaining - (uint64_t)path_len - sizeof expected) break;
        JournalImage *im = calloc(1, sizeof *im);
        if (!im) jinn_persistence_fail("transaction recovery allocation");
        im->path = malloc((size_t)path_len + 1);
        im->data = malloc(len ? (size_t)len : 1);
        im->len = len;
        if (!im->path || !im->data) jinn_persistence_fail("transaction recovery allocation");
        if (fread(im->path, 1, path_len, fp) != path_len ||
            fread(im->data, 1, (size_t)len, fp) != len ||
            fread(&expected, sizeof expected, 1, fp) != 1) {
            if (ferror(fp)) jinn_persistence_fail("transaction journal read");
            free(im->path);
            free(im->data);
            free(im);
            break;
        }
        im->path[path_len] = 0;
        uint64_t h = journal_hash(UINT64_C(14695981039346656037), &path_len, sizeof path_len);
        h = journal_hash(h, &len, sizeof len);
        h = journal_hash(h, im->path, path_len);
        h = journal_hash(h, im->data, (size_t)len);
        if (h != expected || im->path[0] != '/' || strlen(im->path) != path_len)
            jinn_persistence_fail("transaction journal checksum");
        persistence_check_path(im->path);
        im->next = images;
        images = im;
    }
    if (fclose(fp) != 0) jinn_persistence_fail("transaction journal close");
    for (JournalImage *im = images; im; im = im->next)
        if (jinn_atomic_rewrite(im->path, journal_fill, im) != 0)
            jinn_persistence_fail("transaction crash restoration");
    journal_remove();
    while (images) {
        JournalImage *next = images->next;
        free(images->path);
        free(images->data);
        free(images);
        images = next;
    }
}

void jinn_persistence_prepare(void) {
    pthread_mutex_lock(&journal_mu);
    struct stat cwd;
    if (stat(".", &cwd) != 0) jinn_persistence_fail("store directory identity");
    if (journal_ready && (cwd.st_dev != journal_device || cwd.st_ino != journal_inode))
        jinn_persistence_fail("working directory changed after store initialization");
    if (!journal_ready) {
        if (!getcwd(journal_directory, sizeof journal_directory))
            jinn_persistence_fail("store directory path");
        journal_device = cwd.st_dev;
        journal_inode = cwd.st_ino;
        journal_lock_fd = open(".jinn-transaction.lock", O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0600);
        if (journal_lock_fd < 0 || flock(journal_lock_fd, LOCK_EX | LOCK_NB) != 0)
            jinn_persistence_fail("store-directory process exclusion");
        journal_recover();
        journal_ready = 1;
    }
    pthread_mutex_unlock(&journal_mu);
}

static void persistence_check_path(const char *path) {
    if (!path || !*path || strlen(path) >= PATH_MAX) jinn_persistence_fail("store path length");
    char parent[PATH_MAX], resolved[PATH_MAX];
    strcpy(parent, path);
    char *slash = strrchr(parent, '/');
    const char *name = slash ? slash + 1 : path;
    if (!*name || !strcmp(name, ".") || !strcmp(name, "..") ||
        !strncmp(name, ".jinn-transaction", 17))
        jinn_persistence_fail("reserved store path");
    if (slash) {
        if (slash == parent) slash[1] = 0;
        else *slash = 0;
    } else strcpy(parent, ".");
    if (!realpath(parent, resolved) || strcmp(resolved, journal_directory))
        jinn_persistence_fail("store participants must be direct files in the initialized working directory");
    struct stat st;
    if (lstat(path, &st) == 0) {
        if (!S_ISREG(st.st_mode) || st.st_nlink != 1)
            jinn_persistence_fail("store participant must be a regular unaliased file");
    } else if (errno != ENOENT) jinn_persistence_fail("store participant identity");
}

void jinn_persistence_check_path(const char *path) {
    jinn_persistence_prepare();
    persistence_check_path(path);
}

void jinn_txn_journal_snapshot(const char *path, const unsigned char *data, long len) {
    jinn_persistence_prepare();
    persistence_check_path(path);
    if (len < 0) jinn_persistence_fail("transaction snapshot identity");
    char absolute[PATH_MAX];
    if (!realpath(path, absolute)) jinn_persistence_fail("transaction snapshot path");
    if (!journal) {
        journal = fopen(journal_path, "w+bx");
        if (!journal || fwrite(journal_magic, 1, 8, journal) != 8)
            jinn_persistence_fail("transaction journal creation");
        jinn_persistence_sync(journal);
        if (jinn_dir_fsync(journal_path) != 0)
            jinn_persistence_fail("transaction journal directory synchronization");
    }
    uint32_t path_len = (uint32_t)strlen(absolute);
    uint64_t size = (uint64_t)len;
    uint64_t h = journal_hash(UINT64_C(14695981039346656037), &path_len, sizeof path_len);
    h = journal_hash(h, &size, sizeof size);
    h = journal_hash(h, absolute, path_len);
    h = journal_hash(h, data, (size_t)len);
    if (fwrite(&path_len, sizeof path_len, 1, journal) != 1 ||
        fwrite(&size, sizeof size, 1, journal) != 1 ||
        fwrite(absolute, 1, path_len, journal) != path_len ||
        fwrite(data, 1, (size_t)len, journal) != (size_t)len ||
        fwrite(&h, sizeof h, 1, journal) != 1)
        jinn_persistence_fail("transaction undo append");
    jinn_persistence_sync(journal);
}

void jinn_txn_journal_finish(void) {
    if (!journal) return;
    if (fclose(journal) != 0) jinn_persistence_fail("transaction journal close");
    journal = NULL;
    journal_remove();
}

char *jinn_persistence_file_path(FILE *fp) {
    char link[64], path[PATH_MAX];
    int n = snprintf(link, sizeof link, "/proc/self/fd/%d", fileno(fp));
    if (n < 0 || (size_t)n >= sizeof link) jinn_persistence_fail("file identity");
    ssize_t len = readlink(link, path, sizeof path - 1);
    if (len <= 0 || (size_t)len >= sizeof path - 1) jinn_persistence_fail("file identity");
    path[len] = 0;
    char *result = realpath(path, NULL);
    if (!result) jinn_persistence_fail("file identity");
    return result;
}
