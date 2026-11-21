#!/usr/bin/env bash
# Builds libping.so and libping.a into $MOTE_NATIVE_OUT; $MOTE_HEADER_DIR holds mote.h.
set -eu
here=$(dirname "$0")
cc -std=c11 -O2 -fPIC -I "$MOTE_HEADER_DIR" -shared -o "$MOTE_NATIVE_OUT/libping.so" "$here/ping.c"
cc -std=c11 -O2 -fPIC -I "$MOTE_HEADER_DIR" -c -o "$MOTE_NATIVE_OUT/ping.o" "$here/ping.c"
ar rcs "$MOTE_NATIVE_OUT/libping.a" "$MOTE_NATIVE_OUT/ping.o"
rm "$MOTE_NATIVE_OUT/ping.o"
