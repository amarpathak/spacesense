// C interface of the Rust core (ffi/src/lib.rs). Imported into Swift with
// -import-objc-header.
#include <stdint.h>

// phase: 0 reading folders, 1 measuring caches, 2 checking git repositories.
typedef void (*ss_progress_cb)(void *ctx, uint64_t dirs, uint64_t files, uint64_t bytes, uint32_t phase);

// Scans root (NULL = home) and returns JSON {"report":…, "plan":…} or {"error":…}.
// Blocking; call off the main thread. Free the result with ss_free.
char *ss_report(const char *root, ss_progress_cb cb, void *ctx);
// Folder browser: contents of `path` from the last scan, biggest first (max `limit`
// subfolders), each explained. JSON FolderListing or {"error":…}. Free with ss_free.
char *ss_folder(const char *path, uint32_t limit);
void ss_free(char *s);
const char *ss_version(void);
