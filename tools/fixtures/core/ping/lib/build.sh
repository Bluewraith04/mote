#!/usr/bin/env bash
# Builds libping.so and libping.a into $MOTE_LIB_OUT; $MOTE_HEADER_DIR holds mote.h.
set -eu
here=$(dirname "$0")
cc -std=c11 -O2 -fPIC -I "$MOTE_HEADER_DIR" -shared -o "$MOTE_LIB_OUT/libping.so" "$here/ping.c"
cc -std=c11 -O2 -fPIC -I "$MOTE_HEADER_DIR" -c -o "$MOTE_LIB_OUT/ping.o" "$here/ping.c"
ar rcs "$MOTE_LIB_OUT/libping.a" "$MOTE_LIB_OUT/ping.o"
rm "$MOTE_LIB_OUT/ping.o"
