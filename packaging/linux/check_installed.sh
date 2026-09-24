#!/usr/bin/env bash
# Runs an installed iris on a private Xvfb display: the daemon starts,
# maps the home window (WM_CLASS dev.iris.app), and exits on --quit.
# CI runs it in a clean container after the package installs, so a
# library the package does not depend on is missing there.
#
# Needs Xvfb, xdpyinfo, xwininfo, and a Vulkan driver (lavapipe runs
# on a machine without a GPU).
#
#   packaging/linux/check_installed.sh [iris command]
#
# The command defaults to `iris`; for an AppImage pass its path.
set -euo pipefail

[ $# -gt 0 ] || set -- iris

n=99
while xdpyinfo -display ":$n" >/dev/null 2>&1 || [ -e "/tmp/.X$n-lock" ]; do
  n=$((n + 1))
done
Xvfb ":$n" -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
xvfb=$!
home=$(mktemp -d)
trap 'kill "$xvfb" 2>/dev/null || true; rm -rf "$home"' EXIT

# Poll `$1` (a shell condition) every 100 ms for at most $2 seconds.
wait_for() {
  local i
  for ((i = 0; i < $2 * 10; i++)); do
    eval "$1" && return 0
    sleep 0.1
  done
  return 1
}

fail() {
  echo "check_installed: $1" >&2
  cat "$home/state/iris.log" >&2 2>/dev/null || true
  exit 1
}

wait_for 'xdpyinfo -display ":$n" >/dev/null 2>&1' 10 || fail "Xvfb :$n did not start"
mkdir -m 0700 "$home/run"
export DISPLAY=":$n" IRIS_HOME="$home" XDG_RUNTIME_DIR="$home/run"
unset WAYLAND_DISPLAY

# The home window, titled "iris", is mapped.
home_window() {
  local line id=
  while read -r line; do
    case $line in
      *'"iris": ("dev.iris.app" "dev.iris.app")'*)
        id=${line%% *}
        break
        ;;
    esac
  done < <(xwininfo -root -tree)
  [ -n "$id" ] && [[ $(xwininfo -id "$id") == *'Map State: IsViewable'* ]]
}

# The daemon is the one process named iris once the client has exited.
daemon_pid() {
  local d
  for d in /proc/[0-9]*; do
    if [ "$(cat "$d/comm" 2>/dev/null)" = iris ]; then
      echo "${d#/proc/}"
      return 0
    fi
  done
  return 1
}

"$@" --home || fail "iris --home exited with $?"
wait_for home_window 20 || fail "no home window within 20 s"
pid=$(daemon_pid) || fail "no daemon process"
echo "check_installed: daemon $pid mapped the home window"
"$@" --quit || fail "iris --quit exited with $?"
wait_for '! kill -0 "$pid" 2>/dev/null' 10 || fail "the daemon runs 10 s after --quit"
echo "check_installed: the daemon exited on --quit"
