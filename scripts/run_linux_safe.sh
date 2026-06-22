#!/usr/bin/env sh
set -eu

# Compatibility launcher for Linux systems where WebKitGTK cannot create an EGL
# display through the default GPU/Wayland path.
#
# Usage:
#   sh scripts/run_linux_safe.sh
#   DESKBRIDGE_BIN=/path/to/deskbridge sh scripts/run_linux_safe.sh
#   DESKBRIDGE_LINUX_GUI_BACKEND=x11 sh scripts/run_linux_safe.sh

BIN="${DESKBRIDGE_BIN:-./target/release/deskbridge}"

export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"
export WEBKIT_DISABLE_COMPOSITING_MODE="${WEBKIT_DISABLE_COMPOSITING_MODE:-1}"
export LIBGL_ALWAYS_SOFTWARE="${LIBGL_ALWAYS_SOFTWARE:-1}"

if [ "${DESKBRIDGE_LINUX_GUI_BACKEND:-}" = "x11" ]; then
  export GDK_BACKEND=x11
elif [ "${DESKBRIDGE_LINUX_GUI_BACKEND:-}" = "wayland" ]; then
  export GDK_BACKEND=wayland
fi

exec "$BIN" gui
