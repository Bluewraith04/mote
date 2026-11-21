/* A C host of the runtime: sets one up from the byte forms in tables.h, then runs the scenario named on the command line. */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "tables.h"

typedef struct {
    uint64_t region_cursor, region_limit, poll, stack_limit, fault_site, runtime;
} MoteContext;

typedef struct {
    uint32_t kind;
    int64_t site, span_start, span_end;
    const uint8_t *message;
    size_t message_len;
    const uintptr_t *frames;
    size_t frame_count;
} MoteFaultReport;

typedef void (*MoteFaultHandler)(const MoteFaultReport *);

extern const void *mote_runtime_new(const uint8_t *types, size_t types_len, const uint8_t *faults, size_t faults_len, size_t heap_limit);
extern void mote_runtime_free(const void *runtime);
extern void mote_runtime_attach(const void *runtime, MoteContext *context);
extern void mote_runtime_on_fault(const void *runtime, MoteFaultHandler handler);

extern uintptr_t mote_alloc(MoteContext *context, uint32_t type);
extern uintptr_t mote_alloc_array(MoteContext *context, uint32_t type, int64_t count);
extern void mote_fault(MoteContext *context, uint32_t site);
extern void mote_fault_message(MoteContext *context, uintptr_t message);

static MoteContext context;

#ifdef _MSC_VER
#define MOTE_NOINLINE __declspec(noinline)
#else
#define MOTE_NOINLINE __attribute__((noinline))
#endif

static void MOTE_NOINLINE inner(void);
static void MOTE_NOINLINE middle(void);
static void MOTE_NOINLINE outer(void);

/* Whether the return address `pc` lies in the `size` bytes from `function`. */
static int within(uintptr_t pc, void (*function)(void), size_t size) {
    uintptr_t start = (uintptr_t)function;
    return pc > start && pc - start < size;
}

static void on_fault(const MoteFaultReport *report) {
    printf("kind %u\n", report->kind);
    printf("site %lld\n", (long long)report->site);
    printf("span %lld %lld\n", (long long)report->span_start, (long long)report->span_end);
    printf("message %.*s\n", (int)report->message_len, (const char *)report->message);
    printf("fault_site %llu\n", (unsigned long long)context.fault_site);
#ifdef _MSC_VER
    printf("frames %d\n", report->frame_count >= 1 && within(report->frames[0], inner, 256));
#else
    if (report->frame_count >= 4) {
        printf("frames %d %d %d\n", within(report->frames[0], inner, 256), within(report->frames[1], middle, 256), within(report->frames[2], outer, 256));
    } else {
        printf("frames %zu\n", report->frame_count);
    }
#endif
    exit(42);
}

static void MOTE_NOINLINE inner(void) {
    mote_fault(&context, 1);
    abort();
}

static void MOTE_NOINLINE middle(void) {
    inner();
    abort();
}

static void MOTE_NOINLINE outer(void) {
    middle();
    abort();
}

int main(int argc, char **argv) {
    const char *scenario = argc > 1 ? argv[1] : "";
    const void *runtime = mote_runtime_new(types, sizeof types, faults, sizeof faults, 0);
    if (runtime == NULL) {
        printf("tables refused\n");
        return 2;
    }
    mote_runtime_attach(runtime, &context);
    if (strcmp(scenario, "default") != 0) {
        mote_runtime_on_fault(runtime, on_fault);
    }

    if (strcmp(scenario, "alloc") == 0) {
        uintptr_t point = mote_alloc(&context, POINT);
        uintptr_t text = mote_alloc_array(&context, TEXT, 5);
        uint64_t *words = (uint64_t *)point;
        printf("point type %u words %llu %llu\n", (uint32_t)words[0], (unsigned long long)words[1], (unsigned long long)words[2]);
        printf("text type %u length %llu\n", (uint32_t)*(uint64_t *)text, (unsigned long long)((uint64_t *)text)[1]);
        printf("region moved %d\n", context.region_cursor > point);
        mote_runtime_free(runtime);
        return 0;
    }
    if (strcmp(scenario, "fault") == 0) {
        outer();
    }
    if (strcmp(scenario, "default") == 0) {
        mote_fault(&context, 0);
    }
    if (strcmp(scenario, "message") == 0) {
        uintptr_t text = mote_alloc_array(&context, TEXT, 8);
        memcpy((void *)(text + 16), "too deep", 8);
        mote_fault_message(&context, text);
    }
    printf("no scenario\n");
    return 3;
}
