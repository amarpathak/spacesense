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
// Plan from the last scan; never changes anything. request: {"candidates":true}
// (everything cleanable, nothing selected) or {"targets":[…]} (exactly those).
char *ss_plan(const char *request);

// Clean exactly {"targets":[…]} that the user selected AND confirmed. Re-checks
// every step on disk first; logs to ~/Library/Logs/SpaceSense/operations.log.
// Blocking. Returns {"result": CleanResult, "skipped": […]} or {"error":…}.
typedef void (*ss_clean_cb)(void *ctx, uint32_t index, uint32_t total, const char *what);
char *ss_clean(const char *request, ss_clean_cb cb, void *ctx);

void ss_free(char *s);
const char *ss_version(void);
