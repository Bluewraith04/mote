#ifdef _WIN32
#include "pane_win32.h"
#include <windows.h>
#include <commdlg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

void pw_mutex_init(PaneMutex *m) {
    CRITICAL_SECTION *cs = malloc(sizeof(CRITICAL_SECTION));
    InitializeCriticalSection(cs);
    m->ptr = cs;
}

void pw_mutex_lock(PaneMutex *m) {
    if (!m->ptr) {
        CRITICAL_SECTION *cs = malloc(sizeof(CRITICAL_SECTION));
        InitializeCriticalSection(cs);
        if (InterlockedCompareExchangePointer(&m->ptr, cs, NULL) != NULL) {
            DeleteCriticalSection(cs);
            free(cs);
        }
    }
    EnterCriticalSection((CRITICAL_SECTION *)m->ptr);
}

void pw_mutex_unlock(PaneMutex *m) {
    LeaveCriticalSection((CRITICAL_SECTION *)m->ptr);
}

void pw_mutex_destroy(PaneMutex *m) {
    if (m->ptr) {
        DeleteCriticalSection((CRITICAL_SECTION *)m->ptr);
        free(m->ptr);
        m->ptr = NULL;
    }
}

void pw_cond_init(PaneCond *c) {
    CONDITION_VARIABLE *cv = malloc(sizeof(CONDITION_VARIABLE));
    InitializeConditionVariable(cv);
    c->ptr = cv;
}

void pw_cond_wait(PaneCond *c, PaneMutex *m) {
    SleepConditionVariableCS((CONDITION_VARIABLE *)c->ptr, (CRITICAL_SECTION *)m->ptr, INFINITE);
}

void pw_cond_broadcast(PaneCond *c) {
    WakeAllConditionVariable((CONDITION_VARIABLE *)c->ptr);
}

typedef struct ThreadStart {
    void *(*fn)(void *);
    void *arg;
} ThreadStart;

static DWORD WINAPI thread_trampoline(LPVOID param) {
    ThreadStart *start = (ThreadStart *)param;
    void *(*fn)(void *) = start->fn;
    void *arg = start->arg;
    free(start);
    fn(arg);
    return 0;
}

int pw_thread_create(PaneThread *t, void *(*fn)(void *), void *arg) {
    ThreadStart *start = malloc(sizeof(ThreadStart));
    if (!start) return -1;
    start->fn = fn;
    start->arg = arg;
    HANDLE h = CreateThread(NULL, 0, thread_trampoline, start, 0, NULL);
    if (!h) {
        free(start);
        return -1;
    }
    *t = (PaneThread)h;
    return 0;
}

void pw_thread_join(PaneThread t) {
    HANDLE h = (HANDLE)t;
    WaitForSingleObject(h, INFINITE);
    CloseHandle(h);
}

static wchar_t *utf8_to_wide(const char *s) {
    if (!s || !*s) return NULL;
    int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (len <= 0) return NULL;
    wchar_t *w = malloc((size_t)len * sizeof(wchar_t));
    if (!w) return NULL;
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w, len);
    return w;
}

static int64_t copy_wide_to_utf8(const wchar_t *w, uint8_t *out, int64_t out_length) {
    int needed = WideCharToMultiByte(CP_UTF8, 0, w, -1, NULL, 0, NULL, NULL);
    if (needed <= 1) return 0;
    int bytes = needed - 1;
    if (!out || out_length <= 0) return bytes;
    char *tmp = malloc((size_t)needed);
    if (!tmp) return -1;
    WideCharToMultiByte(CP_UTF8, 0, w, -1, tmp, needed, NULL, NULL);
    int64_t copy = bytes < out_length ? bytes : out_length;
    memcpy(out, tmp, (size_t)copy);
    free(tmp);
    return bytes;
}

static void build_filter(const char *ext, wchar_t *buf, size_t cap) {
    // Double-NUL terminated filter, e.g. "CSV (*.csv)\0*.csv\0All Files (*.*)\0*.*\0"
    memset(buf, 0, cap * sizeof(wchar_t));
    if (ext && *ext) {
        wchar_t *wext = utf8_to_wide(ext);
        if (wext) {
            int pos = _snwprintf_s(buf, cap, _TRUNCATE, L"%s Files (*.%s)", wext, wext);
            if (pos >= 0 && (size_t)pos + 1 < cap) {
                pos += 1;
                int p2 = _snwprintf_s(buf + pos, cap - (size_t)pos, _TRUNCATE, L"*.%s", wext);
                if (p2 >= 0) pos += p2 + 1;
                if ((size_t)pos + 20 < cap) {
                    const wchar_t all[] = L"All Files (*.*)\0*.*\0";
                    memcpy(buf + pos, all, sizeof(all));
                }
            }
            free(wext);
            return;
        }
    }
    const wchar_t all[] = L"All Files (*.*)\0*.*\0";
    memcpy(buf, all, sizeof(all));
}

__declspec(dllexport) int64_t pane_dialog_open(const char *title, const char *filter_ext, uint8_t *out, int64_t out_length) {
    wchar_t file_buf[4096] = {0};
    wchar_t filter_buf[256];
    build_filter(filter_ext, filter_buf, 256);
    wchar_t *wtitle = utf8_to_wide(title);
    OPENFILENAMEW ofn = {0};
    ofn.lStructSize = sizeof(ofn);
    ofn.lpstrFilter = filter_buf;
    ofn.lpstrFile = file_buf;
    ofn.nMaxFile = (DWORD)(sizeof(file_buf) / sizeof(wchar_t));
    ofn.lpstrTitle = wtitle;
    ofn.Flags = OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST;
    BOOL ok = GetOpenFileNameW(&ofn);
    free(wtitle);
    if (!ok) return 0;
    return copy_wide_to_utf8(file_buf, out, out_length);
}

__declspec(dllexport) int64_t pane_dialog_save(const char *title, const char *filter_ext, const char *default_name, uint8_t *out, int64_t out_length) {
    wchar_t file_buf[4096] = {0};
    if (default_name && *default_name) {
        MultiByteToWideChar(CP_UTF8, 0, default_name, -1, file_buf, 4096);
    }
    wchar_t filter_buf[256];
    build_filter(filter_ext, filter_buf, 256);
    wchar_t *wtitle = utf8_to_wide(title);
    wchar_t *wext = utf8_to_wide(filter_ext);
    OPENFILENAMEW ofn = {0};
    ofn.lStructSize = sizeof(ofn);
    ofn.lpstrFilter = filter_buf;
    ofn.lpstrFile = file_buf;
    ofn.nMaxFile = (DWORD)(sizeof(file_buf) / sizeof(wchar_t));
    ofn.lpstrTitle = wtitle;
    ofn.lpstrDefExt = wext;
    ofn.Flags = OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT;
    BOOL ok = GetSaveFileNameW(&ofn);
    free(wtitle);
    free(wext);
    if (!ok) return 0;
    return copy_wide_to_utf8(file_buf, out, out_length);
}
#else
typedef int pane_win32_unused;
#endif
