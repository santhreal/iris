#!/usr/bin/env bash
# Runs an installed iris on a private Xvfb display: the daemon starts,
# maps the home window (WM_CLASS dev.iris.app), and exits on --quit.
# CI runs it in a clean container after the package installs, so a
# library the package does not depend on is missing there.
#
# With --upgrade, the upgrade command runs while the daemon runs. The
# daemon keeps running on the file the upgrade replaced, exits on
# --quit from the upgraded iris, and the upgraded iris starts again.
#
# Needs Xvfb, xdpyinfo, xwininfo, and a Vulkan driver (lavapipe runs
# on a machine without a GPU).
#
#   packaging/linux/check_installed.sh [--upgrade <command>] [iris command]
#
# The iris command defaults to `iris`; for an AppImage pass its path.
set -euo pipefail

upgrade=
if [ "${1:-}" = --upgrade ]; then
  upgrade=$2
  shift 2
fi
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

# The daemon is the one running process named iris with this check's
# IRIS_HOME once the client has exited; an iris of the desktop session
# has another. A daemon that exited and is not yet reaped keeps its
# name but has no binary.
daemon_pid() {
  local d
  for d in /proc/[0-9]*; do
    if [ "$(cat "$d/comm" 2>/dev/null)" = iris ] && readlink "$d/exe" >/dev/null 2>&1 \
      && grep -qxzF "IRIS_HOME=$home" "$d/environ" 2>/dev/null; then
      echo "${d#/proc/}"
      return 0
    fi
  done
  return 1
}

# Start the daemon through --home; sets $pid once the window maps.
start() {
  "$@" --home || fail "iris --home exited with $?"
  wait_for home_window 20 || fail "no home window within 20 s"
  pid=$(daemon_pid) || fail "no daemon process"
  echo "check_installed: daemon $pid mapped the home window"
}

# The daemon's binary; fails once the daemon has exited, as a zombie
# has no binary either.
daemon_exe() {
  readlink "/proc/$pid/exe" 2>/dev/null
}

quit() {
  "$@" --quit || fail "iris --quit exited with $?"
  wait_for '! daemon_exe >/dev/null' 10 || fail "the daemon runs 10 s after --quit"
  wait_for '! home_window' 10 || fail "the home window is mapped 10 s after the daemon exited"
  echo "check_installed: the daemon exited on --quit"
}

start "$@"
if [ -n "$upgrade" ]; then
  bash -c "$upgrade" || fail "the upgrade exited with $?"
  exe=$(daemon_exe) || fail "the daemon exited during the upgrade"
  [[ $exe == *' (deleted)' ]] || fail "the upgrade left the daemon's binary in place: $exe"
  echo "check_installed: daemon $pid runs on after the upgrade replaced $exe"
  quit "$@"
  old=$pid
  start "$@"
  [ "$pid" != "$old" ] || fail "the daemon after the upgrade is the one it replaced, $pid"
  [[ $(daemon_exe) != *' (deleted)' ]] || fail "the daemon after the upgrade runs the replaced binary"
fi
quit "$@"
