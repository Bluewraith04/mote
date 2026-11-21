// The shared library the FFI tests link: one small function for each way a value crosses to C.
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#ifdef DEMO_DLL
#define DEMO_EXPORT __declspec(dllexport)
#else
#define DEMO_EXPORT
#endif
#else
#include <pthread.h>
#define DEMO_EXPORT
#endif

typedef struct MotePort MotePort;

typedef struct MoteHost {
    uint64_t size;
    int32_t (*port_push)(MotePort *, const uint8_t *, int64_t);
    void (*port_close)(MotePort *);
} MoteHost;

static uint64_t seen_size;
static MoteHost host_table;

DEMO_EXPORT void mote_init(const MoteHost *host) {
    seen_size = host->size;
    host_table = *host;
}
DEMO_EXPORT uint64_t host_size(void) { return seen_size; }

DEMO_EXPORT int64_t add_i64(int64_t a, int64_t b) { return a + b; }
DEMO_EXPORT int32_t add_i32(int32_t a, int32_t b) { return a + b; }
DEMO_EXPORT int16_t neg_i16(int16_t a) { return (int16_t)-a; }
DEMO_EXPORT int8_t neg_i8(int8_t a) { return (int8_t)-a; }
DEMO_EXPORT uint8_t inc_u8(uint8_t a) { return (uint8_t)(a + 1); }
DEMO_EXPORT uint16_t inc_u16(uint16_t a) { return (uint16_t)(a + 1); }
DEMO_EXPORT uint32_t inc_u32(uint32_t a) { return a + 1; }
DEMO_EXPORT uint64_t half_u64(uint64_t a) { return a / 2; }

DEMO_EXPORT int64_t widen_i8(int8_t a) { return a; }
DEMO_EXPORT int64_t widen_u8(uint8_t a) { return a; }
DEMO_EXPORT int64_t widen_i16(int16_t a) { return a; }
DEMO_EXPORT int64_t widen_u16(uint16_t a) { return a; }
DEMO_EXPORT int64_t widen_i32(int32_t a) { return a; }
DEMO_EXPORT int64_t widen_u32(uint32_t a) { return a; }

DEMO_EXPORT double squares(double a, double b) { return a * a + b * b; }
DEMO_EXPORT float halve(float a) { return a / 2.0f; }
DEMO_EXPORT uint8_t is_even(int64_t n) { return n % 2 == 0; }

DEMO_EXPORT int64_t length_of(const char *s) { return (int64_t)strlen(s); }
DEMO_EXPORT int64_t first_byte(const char *s) { return (unsigned char)s[0]; }
DEMO_EXPORT const char *greeting(void) { return "hello from c"; }
DEMO_EXPORT const char *nothing(void) { return NULL; }

DEMO_EXPORT int64_t sum_ints(const int64_t *items, int64_t n) {
    int64_t total = 0;
    for (int64_t i = 0; i < n; i++) total += items[i];
    return total;
}

DEMO_EXPORT double sum_floats(const double *items, int64_t n) {
    double total = 0;
    for (int64_t i = 0; i < n; i++) total += items[i];
    return total;
}

DEMO_EXPORT int64_t count_set(const uint8_t *items, int64_t n) {
    int64_t total = 0;
    for (int64_t i = 0; i < n; i++) total += items[i] != 0;
    return total;
}

typedef struct Cell {
    int64_t value;
} Cell;

DEMO_EXPORT void *cell_new(int64_t value) {
    Cell *cell = malloc(sizeof(Cell));
    cell->value = value;
    return cell;
}

DEMO_EXPORT int64_t cell_get(void *cell) { return ((Cell *)cell)->value; }
DEMO_EXPORT void *cell_none(void) { return NULL; }
DEMO_EXPORT void *cell_find(int64_t key) { return key == 7 ? cell_new(70) : NULL; }
DEMO_EXPORT int64_t cell_or(void *cell, int64_t otherwise) { return cell ? ((Cell *)cell)->value : otherwise; }

DEMO_EXPORT void bump_i32(int32_t *x) { *x += 1; }
DEMO_EXPORT void swap_f64(double *a, double *b) {
    double t = *a;
    *a = *b;
    *b = t;
}
DEMO_EXPORT void set_flag(uint8_t *flag, int64_t on) { *flag = on != 0; }
DEMO_EXPORT void fresh_cell(void **cell) { *cell = cell_new(5); }
DEMO_EXPORT void clear_cell(void **cell) { *cell = NULL; }

#ifdef _WIN32
DEMO_EXPORT int64_t thread_tag(void) { return (int64_t)GetCurrentThreadId(); }
#else
DEMO_EXPORT int64_t thread_tag(void) { return (int64_t)pthread_self(); }
#endif

typedef struct Feeding {
    MotePort *port;
    int64_t count;
} Feeding;

// Pushes `count` messages, the nth of n + 1 bytes of n, then gives the port back.
#ifdef _WIN32
static DWORD WINAPI feed_thread(LPVOID argument) {
#else
static void *feed_thread(void *argument) {
#endif
    Feeding *feeding = argument;
    uint8_t bytes[64];
    for (int64_t n = 0; n < feeding->count; n++) {
        memset(bytes, (int)n, sizeof bytes);
        host_table.port_push(feeding->port, bytes, n + 1);
    }
    host_table.port_close(feeding->port);
    free(feeding);
#ifdef _WIN32
    return 0;
#else
    return NULL;
#endif
}

DEMO_EXPORT void feed_port(MotePort *port, int64_t count) {
    Feeding *feeding = malloc(sizeof(Feeding));
    feeding->port = port;
    feeding->count = count;
#ifdef _WIN32
    HANDLE thread = CreateThread(NULL, 0, feed_thread, feeding, 0, NULL);
    if (thread != NULL) CloseHandle(thread);
#else
    pthread_t thread;
    pthread_create(&thread, NULL, feed_thread, feeding);
    pthread_detach(thread);
#endif
}

// Pushes a message of `length` bytes and gives the result: 0 queued, 1 full, 2 closed.
DEMO_EXPORT int64_t push_status(MotePort *port, int64_t length) {
    uint8_t bytes[64] = {0};
    return host_table.port_push(port, bytes, length);
}

DEMO_EXPORT void release_port(MotePort *port) { host_table.port_close(port); }
