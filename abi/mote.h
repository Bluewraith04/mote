/* The host table of a Mote external library. Generated from abi/src/host.rs; do not edit. */
#ifndef MOTE_H
#define MOTE_H

#include <stddef.h>
#include <stdint.h>

/* The runtime's end of a port. */
typedef struct MotePort MotePort;

#define MOTE_PUSH_QUEUED 0 /* port_push: the message was queued */
#define MOTE_PUSH_FULL 1 /* port_push: the port is full and the message was not taken */
#define MOTE_PUSH_CLOSED 2 /* port_push: the receiver was closed or dropped */
#define MOTE_WAIT_READABLE 0 /* fd_wait interest: the descriptor can be read */
#define MOTE_WAIT_WRITABLE 1 /* fd_wait interest: the descriptor can be written */
#define MOTE_WAIT_READY 0 /* fd_wait: the descriptor is ready */
#define MOTE_WAIT_TIMED_OUT 1 /* fd_wait: the time ran out first */
#define MOTE_WAIT_FAILED 2 /* fd_wait: the descriptor is invalid or in error */

/* The table `mote_init` receives; a library reads only the fields inside `size`. */
typedef struct MoteHost {
    uint64_t size;
    /* Copies a message into the port; callable from any thread, never waits. */
    int32_t (*port_push)(MotePort *port, const uint8_t *bytes, int64_t length);
    /* Ends the sender and releases the reference the library was given. */
    void (*port_close)(MotePort *port);
    /* Blocks the calling thread until the descriptor is ready or the time is out (negative waits without limit). */
    int32_t (*fd_wait)(int32_t fd, int32_t interest, int64_t timeout_ms);
} MoteHost;

/* Called once when the library is loaded. */
void mote_init(const MoteHost *host);

#endif
