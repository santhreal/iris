#!/usr/bin/env bash
# release-prep.sh: prepares the release commit for one version.
#
# Usage:
#   scripts/release-prep.sh VERSION
#
# VERSION is a semantic version without the tag's leading v: 0.2.0, or
# 0.2.0-beta.1 for a prerelease. Run it in a checkout of main with no
# local changes.
#
# It sets the [package] version in Cargo.toml, updates the iris entry
# in Cargo.lock with `cargo metadata`, renames the CHANGELOG.md heading
# "## [Unreleased]" to "## [VERSION] - YYYY-MM-DD" (today, UTC), and
# adds an empty "## [Unreleased]" section above it. It commits, tags,
# and pushes nothing; it prints the commands that do.
#
# It changes nothing and exits with status 1 when VERSION is not a
# semantic version, the working tree has uncommitted or untracked
# files, CHANGELOG.md has no "## [Unreleased]" section or one without
# entries, or CHANGELOG.md already has a section for VERSION.
set -euo pipefail
export LC_ALL=C

show_help() {
  awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; seen = 1; next } seen { exit }' "$0"
}

die() {
  printf 'Error: %s\n' "$1" >&2
  exit 1
}

case "${1:-}" in
  -h | --help)
    show_help
    exit 0
    ;;
esac
if [[ $# -ne 1 ]]; then
  show_help >&2
  exit 2
fi
version=$1

# Semantic Versioning 2.0.0, without build metadata: semver precedence
# ignores build metadata, so the updater could not order two releases
# that differ only in it.
ident='(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
semver="^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-$ident(\.$ident)*)?$"
if [[ $version == v* ]]; then
  die "'$version' starts with v; pass ${version#v}, the tag adds the v"
fi
if [[ $version == *+* ]]; then
  die "'$version' has build metadata (+...); release versions take none"
fi
if [[ ! $version =~ $semver ]]; then
  die "'$version' is not a semantic version MAJOR.MINOR.PATCH[-PRERELEASE], such as 0.2.0 or 0.2.0-beta.1"
fi

root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "$(pwd) is not in a git checkout; run release-prep.sh in the iris checkout"
cd "$root"

for file in Cargo.toml Cargo.lock CHANGELOG.md; do
  [[ -f $file ]] || die "$root/$file does not exist"
done

# package_field FIELD: prints the string value of FIELD in the
# [package] table of Cargo.toml.
package_field() {
  awk -v field="$1" '
    /^\[/ { in_package = ($0 == "[package]"); next }
    in_package && index($0, field " = \"") == 1 {
      sub(/^[^"]*"/, ""); sub(/".*$/, ""); print; exit
    }
  ' Cargo.toml
}

name=$(package_field name)
[[ $name == iris ]] || die "$root/Cargo.toml is the package '$name', not iris"
current=$(package_field version)
[[ -n $current ]] || die "$root/Cargo.toml has no version in its [package] table"

dirty=$(git status --porcelain --untracked-files=normal)
if [[ -n $dirty ]]; then
  printf '%s\n' "$dirty" >&2
  die "the working tree has the changes listed above; commit or remove them, then run release-prep.sh again"
fi

# awk reads the pattern from ENVIRON: a -v assignment would process its
# backslashes as string escapes.
export UNRELEASED='^## \[Unreleased\][[:space:]]*$'
headings=$(grep -c "$UNRELEASED" CHANGELOG.md || true)
case "$headings" in
  0) die "CHANGELOG.md has no '## [Unreleased]' section" ;;
  1) ;;
  *) die "CHANGELOG.md has $headings '## [Unreleased]' headings; merge them into one" ;;
esac

# An entry is any line of the section other than a blank line or a
# "### Added"-style subheading.
entries=$(awk '
  /^## / { in_section = ($0 ~ ENVIRON["UNRELEASED"]); next }
  in_section && NF && !/^### / { n++ }
  END { print n + 0 }
' CHANGELOG.md)
if [[ $entries -eq 0 ]]; then
  die "the '## [Unreleased]' section of CHANGELOG.md has no entries; add the changes in $version under it"
fi

if awk -v heading="## [$version]" 'index($0, heading) == 1 { found = 1 } END { exit !found }' CHANGELOG.md; then
  die "CHANGELOG.md already has a section for $version; a released version is never released again, choose the next one"
fi

today=$(date -u +%Y-%m-%d)

# Every edit is staged in a directory under .git, and the three files
# are restored from it unless the run completes.
work=$(mktemp -d "$(git rev-parse --absolute-git-dir)/release-prep.XXXXXX")
complete=0
cleanup() {
  if [[ $complete -eq 0 ]]; then
    cp "$work/orig/Cargo.toml" "$work/orig/Cargo.lock" "$work/orig/CHANGELOG.md" "$root/" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
mkdir "$work/orig"
cp Cargo.toml Cargo.lock CHANGELOG.md "$work/orig/"

awk -v version="$version" '
  /^\[/ { in_package = ($0 == "[package]") }
  in_package && !done && /^version = "/ { print "version = \"" version "\""; done = 1; next }
  { print }
' Cargo.toml >"$work/Cargo.toml"

awk -v heading="## [$version] - $today" '
  $0 ~ ENVIRON["UNRELEASED"] { print "## [Unreleased]"; print ""; print heading; next }
  { print }
' CHANGELOG.md >"$work/CHANGELOG.md"

cp "$work/Cargo.toml" Cargo.toml
[[ $(package_field version) == "$version" ]] || die "Cargo.toml does not hold version $version after the edit"

# cargo metadata resolves the workspace and rewrites Cargo.lock with the
# new version of the iris package.
if ! cargo metadata --format-version 1 >/dev/null; then
  die "cargo metadata failed; Cargo.toml and Cargo.lock are restored"
fi
locked=$(awk '
  $0 == "[[package]]" { name = ""; next }
  /^name = "/ { name = $0; next }
  name == "name = \"iris\"" && /^version = "/ { sub(/^version = "/, ""); sub(/"$/, ""); print; exit }
' Cargo.lock)
[[ $locked == "$version" ]] ||
  die "Cargo.lock lists iris ${locked:-nowhere} after cargo metadata, not $version; Cargo.toml and Cargo.lock are restored"

cp "$work/CHANGELOG.md" CHANGELOG.md
complete=1

tag="v$version"
printf 'Prepared iris %s (Cargo.toml had %s):\n' "$version" "$current"
printf '  Cargo.toml    version = "%s"\n' "$version"
printf '  Cargo.lock    iris %s\n' "$version"
printf '  CHANGELOG.md  ## [%s] - %s\n' "$version" "$today"
git --no-pager diff --stat
if [[ $version == *-* ]]; then
  printf '\n%s is a prerelease: the release workflow publishes it as a GitHub prerelease, which the beta update channel offers.\n' "$tag"
fi
cat <<EOF

Review the change, then commit it and push main:
  git diff
  git commit -m "Release $version" -- Cargo.toml Cargo.lock CHANGELOG.md
  git push origin main

When CI passes on that commit, tag it and push the tag:
  git tag -a $tag -m "iris $version"
  git push origin $tag
EOF
