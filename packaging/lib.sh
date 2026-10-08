# shellcheck shell=bash
# Functions shared by the packaging scripts. Source this file; it runs
# nothing on its own.
#
# Version forms (packaging/CONTRACT.md, "Versions"):
#   cargo  0.2.0-beta.1   asset names, `iris --version`, display strings
#   tilde  0.2.0~beta.1   deb and rpm Version, so a prerelease sorts
#                         before its release
#   core   0.2.0          macOS bundle versions, Windows numeric versions

# iris_cargo_version REPO_ROOT
# Prints the package version in REPO_ROOT/Cargo.toml.
iris_cargo_version() {
  sed -nE 's/^version = "([^"]+)"/\1/p' "$1/Cargo.toml" | head -n1
}

# iris_tilde_version VERSION
# Prints VERSION with its prerelease separator `-` as `~`. Fails when
# another `-` remains in the prerelease or the build metadata: a deb
# Version reads it as the start of the Debian revision, and an rpm
# Version cannot hold it.
iris_tilde_version() {
  local main=${1%%+*} build=''
  if [[ "$1" == *+* ]]; then
    build=+${1#*+}
  fi
  main=${main/-/\~}
  if [[ "$main$build" == *-* ]]; then
    echo "Error: version '$1' holds a '-' after its prerelease separator; deb and rpm versions cannot hold it" >&2
    return 1
  fi
  printf '%s\n' "$main$build"
}

# iris_core_version VERSION
# Prints major.minor.patch of VERSION, without prerelease or build
# metadata. Fails when VERSION does not start with three numbers.
iris_core_version() {
  local core=${1%%[-+]*}
  if [[ ! "$core" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: version '$1' is not major.minor.patch[-prerelease]" >&2
    return 1
  fi
  printf '%s\n' "$core"
}

# iris_binary_arch FILE
# Prints the architecture of an ELF or PE executable in the
# std::env::consts::ARCH form: x86_64 or aarch64. Fails on any other
# file or machine type, on a 32-bit or big-endian ELF, and on a PE
# whose e_lfanew does not point at the PE signature.
iris_binary_arch() {
  local file=$1 magic machine='' pe
  magic=$(od -An -tx1 -N6 "$file" | tr -d ' \n')
  case "$magic" in
    7f454c460201)
      # ELFCLASS64, ELFDATA2LSB. e_machine at offset 18: 62
      # (EM_X86_64), 183 (EM_AARCH64).
      machine=$(od -An -tu2 -j18 -N2 "$file" 2>/dev/null | tr -d ' \n')
      case "$machine" in
        62) echo x86_64; return ;;
        183) echo aarch64; return ;;
      esac
      ;;
    4d5a*)
      # PE: e_lfanew at 0x3C locates "PE\0\0", followed by Machine:
      # 0x8664 (AMD64), 0xAA64 (ARM64).
      pe=$(od -An -tu4 -j60 -N4 "$file" 2>/dev/null | tr -d ' \n')
      if [[ -n "$pe" && "$(od -An -tx1 -j"$pe" -N4 "$file" 2>/dev/null | tr -d ' \n')" == 50450000 ]]; then
        machine=$(od -An -tu2 -j$((pe + 4)) -N2 "$file" | tr -d ' \n')
      fi
      case "$machine" in
        34404) echo x86_64; return ;;
        43620) echo aarch64; return ;;
      esac
      ;;
  esac
  echo "Error: $file is not an x86_64 or aarch64 ELF or PE executable (magic $magic, machine ${machine:-none})" >&2
  return 1
}

# iris_resolve_arch FILE [ARCH]
# Prints the architecture of FILE. With ARCH, fails unless ARCH is
# x86_64 or aarch64 (amd64 and arm64 name the same) and FILE is built
# for it.
iris_resolve_arch() {
  local file=$1 want=${2:-} have
  case "$want" in
    '' | x86_64 | aarch64) ;;
    amd64) want=x86_64 ;;
    arm64) want=aarch64 ;;
    *)
      echo "Error: --arch $want is not x86_64 or aarch64" >&2
      return 1
      ;;
  esac
  have=$(iris_binary_arch "$file") || return 1
  if [[ -n "$want" && "$want" != "$have" ]]; then
    echo "Error: --arch $want, but $file is built for $have" >&2
    return 1
  fi
  printf '%s\n' "$have"
}

# iris_help SCRIPT
# Prints the comment block that follows the first line of SCRIPT.
iris_help() {
  awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; seen = 1; next } seen { exit }' "$1"
}

# iris_staging_dir REPO_ROOT NAME
# Creates a directory named NAME.XXXXXX under REPO_ROOT/.build-staging
# (gitignored) and prints its path.
iris_staging_dir() {
  mkdir -p "$1/.build-staging"
  mktemp -d "$1/.build-staging/$2.XXXXXX"
}

# iris_license_files REPO_ROOT DIR
# Copies the license notices every package ships into DIR:
# LICENSE-MIT and LICENSE-APACHE (iris), and Inter-OFL.txt (the SIL
# Open Font License 1.1 of the Inter font the binary embeds).
iris_license_files() {
  cp "$1/LICENSE-MIT" "$1/LICENSE-APACHE" "$1/assets/fonts/Inter-OFL.txt" "$2/"
}

# iris_sha256_sidecar FILE
# Writes FILE.sha256 in the format of sha256sum: the hex digest, two
# spaces, and the file name without its directory.
iris_sha256_sidecar() {
  local dir name
  dir=$(dirname "$1")
  name=$(basename "$1")
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$dir" && sha256sum "$name" >"$name.sha256")
  else
    (cd "$dir" && shasum -a 256 "$name" >"$name.sha256")
  fi
}

# iris_hicolor_icons REPO_ROOT DIR
# Installs packaging/icons/iris-{size}.png as DIR/{size}x{size}/apps/iris.png
# for an icon theme directory such as usr/share/icons/hicolor, for every
# size from 16 to 1024 px. Fails when one of the icons is missing.
iris_hicolor_icons() {
  local s
  for s in 16 24 32 48 64 128 256 512 1024; do
    install -D -m 0644 "$1/packaging/icons/iris-${s}.png" "$2/${s}x${s}/apps/iris.png"
  done
}
