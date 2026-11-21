#!/usr/bin/env bash
# Builds libpane.so and libpane.a into $MOTE_NATIVE_OUT from the C sources here and the vendored clay.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
# Raylib takes a minute to build, so builds take turns and both outputs newer than every source are kept.
exec 9>"$MOTE_NATIVE_OUT/.build.lock"
flock 9
if [ -f "$MOTE_NATIVE_OUT/libpane.so" ] && [ -f "$MOTE_NATIVE_OUT/libpane.a" ] && [ -z "$(find "$here" -path "$MOTE_NATIVE_OUT" -prune -o -type f -newer "$MOTE_NATIVE_OUT/libpane.a" -print -quit)" ]; then exit 0; fi
cache=$(mktemp -d)
trap 'rm -rf "$cache"' EXIT
raylib="$here/vendor/raylib/src"
own=("$here/pane_layout.c" "$here/pane_window.c")
raylib_sources=("$raylib/rcore.c" "$raylib/rshapes.c" "$raylib/rtext.c" "$raylib/rtextures.c")
own_flags=(-cflags -std=gnu11 -fvisibility=hidden "-DPANE_FONT_PATH=\"$here/vendor/font/DejaVuSansMono.ttf\"" --)
raylib_flags=(-cflags -std=gnu11 -fvisibility=hidden -w -DPLATFORM_DESKTOP -DGRAPHICS_API_OPENGL_33 -DSUPPORT_MODULE_RMODELS=0 -DSUPPORT_MODULE_RAUDIO=0 "-I$raylib" "-I$raylib/external/glfw/include" --)
# GLFW is X11 only, and only its own file sees `_GLFW_X11`, so raylib does not call Xlib itself.
glfw_flags=(-cflags -std=gnu11 -fvisibility=hidden -w -D_GLFW_X11 "-I$raylib" "-I$raylib/external/glfw/include" --)
sources=("${own_flags[@]}" "${own[@]}" "${raylib_flags[@]}" "${raylib_sources[@]}" "${glfw_flags[@]}" "$raylib/rglfw.c")
zig build-lib "${sources[@]}" --name pane -I "$here" -lc -dynamic -OReleaseFast -fPIC -fstrip --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$cache/libpane.so" -fno-emit-h
zig build-lib "${sources[@]}" --name pane -I "$here" -lc -static -OReleaseFast -fPIC -fstrip --cache-dir "$cache" --global-cache-dir "$cache" -femit-bin="$cache/libpane.a" -fno-emit-h
# The archive keeps only its `pane_` symbols and the init global, so clay and raylib cannot clash with the program.
ld -r --whole-archive "$cache/libpane.a" -o "$cache/pane.o"
objcopy -w --keep-global-symbol='pane_*' --keep-global-symbol='mote_init_pane' --localize-symbol=mote_init "$cache/pane.o"
ar rcs "$cache/archive.a" "$cache/pane.o"
# Each file is renamed into place, so a build running at the same time never sees half of one.
cp "$cache/libpane.so" "$MOTE_NATIVE_OUT/.libpane.so.$$"
mv -f "$MOTE_NATIVE_OUT/.libpane.so.$$" "$MOTE_NATIVE_OUT/libpane.so"
cp "$cache/archive.a" "$MOTE_NATIVE_OUT/.libpane.a.$$"
mv -f "$MOTE_NATIVE_OUT/.libpane.a.$$" "$MOTE_NATIVE_OUT/libpane.a"
