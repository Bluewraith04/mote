#include "mote.h"

#ifdef _WIN32
#ifdef PING_DLL
#define PING_EXPORT __declspec(dllexport)
#else
#define PING_EXPORT
#endif
#else
#define PING_EXPORT
#endif

PING_EXPORT int32_t ping_add(int32_t a, int32_t b) {
    return a + b;
}
