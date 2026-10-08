# Releasing iris

A release is a tag `v{version}` pushed to `santhreal/iris`. The tag starts `.github/workflows/release.yml`, which builds, checks, signs, and publishes every asset on GitHub Releases. Nothing else publishes a release.

## Versions

The version in `Cargo.toml` is a [Semantic Versioning](https://semver.org/spec/v2.0.0.html) version without build metadata:

- A stable release is `MAJOR.MINOR.PATCH`, such as `0.2.0`. It publishes as a GitHub release and becomes `releases/latest`, which the `stable` update channel reads.
- A beta is a prerelease version, such as `0.2.0-beta.1`, `0.2.0-beta.2`, then `0.2.0`. It publishes as a GitHub prerelease. `releases/latest` stays on the last stable release, and only the `beta` update channel offers it ([Updates](docs/updates.md)).

The tag is `v` followed by the version: `v0.2.0`, `v0.2.0-beta.1`. Asset names hold the version unchanged (`iris-0.2.0-beta.1-linux-x86_64.deb`). The packaging scripts derive the deb, rpm, macOS bundle, and Windows resource version forms from it; `packaging/CONTRACT.md` lists them.

A pushed tag is never deleted or moved, and a published release is never deleted and its assets are never replaced. To correct a release, release the next version: the next patch (`0.2.1`) or the next prerelease (`0.2.0-beta.2`).

## Prerequisites

- Push access to `main` and to tags on `santhreal/iris`.
- `git`, `bash`, and `minisign`.
- The GitHub CLI `gh`, signed in to an account with access to the repository (`gh auth status`).
- `rustup` and the toolchain `rust-toolchain.toml` pins (Rust 1.98.0): run `rustup toolchain install` in the checkout. `scripts/release-prep.sh` runs `cargo metadata`.
- The repository secret `MINISIGN_SECRET_KEY` holds the release signing key: the base64 encoding of the unencrypted minisign secret key file. The publish job of `release.yml` signs every asset with it. The matching public key is `packaging/minisign.pub`, key id `EC8B4C1097043E88`:

  ```
  RWSIPgSXEEyL7E0nxtQR83kxxdTJGm0GcQ1/J/EOUgG+cCDn9RJjIYox
  ```

## 1. Prepare the release commit

Check out `main` at `origin/main` with no local changes:

```sh
git switch main
git pull --ff-only origin main
git status --short
```

`git status --short` prints nothing on a tree with no local changes.

Read the `## [Unreleased]` section of `CHANGELOG.md`. It lists every user-visible change since the last release, one sentence per entry, under `### Added`, `### Changed`, `### Fixed`, `### Removed`, or `### Security`. Commit any missing entry to `main` first.

Run the release preparation script with the new version:

```sh
scripts/release-prep.sh 0.2.0
```

The script sets the version in `Cargo.toml`, updates the `iris` entry in `Cargo.lock` with `cargo metadata`, renames `## [Unreleased]` to `## [0.2.0] - YYYY-MM-DD` with the current UTC date, and adds an empty `## [Unreleased]` section above it. It commits, tags, and pushes nothing, and prints the commands of the next steps.

It changes nothing and exits with status 1 when:

- the version is not `MAJOR.MINOR.PATCH` or `MAJOR.MINOR.PATCH-PRERELEASE`, starts with `v`, or has build metadata;
- the working tree has uncommitted changes or untracked files;
- `CHANGELOG.md` has no `## [Unreleased]` section, or the section holds no entries;
- `CHANGELOG.md` already has a section for the version.

## 2. Commit and push

```sh
git diff
git commit -m "Release 0.2.0" -- Cargo.toml Cargo.lock CHANGELOG.md
git push origin main
```

## 3. Wait for CI

Find the CI run of the release commit and watch it to the end:

```sh
gh run list --repo santhreal/iris --workflow ci.yml --commit "$(git rev-parse HEAD)"
gh run watch --repo santhreal/iris --exit-status <run-id>
```

`gh run watch --exit-status` exits with status 0 only when every job passed. When a job fails, fix the cause in a new commit on `main`, push it, and wait for its CI run. The version and the `CHANGELOG.md` section stay as they are until the tag is pushed.

To build, package, install-check, sign, and verify every asset without publishing, run the release workflow by hand. A run started this way creates no tag, release, or draft, and reports a missing `CHANGELOG.md` section as a warning:

```sh
gh workflow run release.yml --repo santhreal/iris --ref main
gh run list --repo santhreal/iris --workflow release.yml --limit 1
gh run watch --repo santhreal/iris --exit-status <run-id>
```

## 4. Tag the commit and push the tag

Tag the commit whose CI run passed:

```sh
git tag -a v0.2.0 -m "iris 0.2.0" <commit>
git push origin v0.2.0
```

`release.yml` fails without publishing when the tag is not `v` followed by the `Cargo.toml` version, when the tagged commit is not on `origin/main`, or when `CHANGELOG.md` has no `## [0.2.0] - YYYY-MM-DD` heading. It then runs the test jobs CI runs and builds and install-checks every package, checks the asset set against `packaging/CONTRACT.md`, signs each asset, verifies each signature against `packaging/minisign.pub`, and publishes the release `iris 0.2.0`. The text between the version's `CHANGELOG.md` heading and the next `## ` heading is the release body. A version with a prerelease part publishes as a GitHub prerelease.

## 5. Watch the release workflow

```sh
gh run list --repo santhreal/iris --workflow release.yml --limit 1
gh run watch --repo santhreal/iris --exit-status <run-id>
```

A run that fails publishes nothing. When the failure is in the runner and not in iris, such as a network error during a download, rerun the failed jobs:

```sh
gh run rerun <run-id> --failed --repo santhreal/iris
```

Any other failure needs a fix on `main` and the next version. The tag stays where it is.

## 6. Verify the release

Check the release's type and state:

```sh
gh release view v0.2.0 --repo santhreal/iris --json tagName,isDraft,isPrerelease
```

`isDraft` is `false`. `isPrerelease` is `false` for a stable version and `true` for a prerelease.

Download every asset into an empty directory under `dist/` (ignored by git):

```sh
mkdir -p dist/v0.2.0
cd dist/v0.2.0
gh release download v0.2.0 --repo santhreal/iris
ls
```

The release holds 33 files: these 11 assets, each with a `.sha256` and a `.minisig` file.

| Asset |
| --- |
| `iris-{ver}-windows-x86_64-setup.exe` |
| `iris-{ver}-windows-x86_64-portable.zip` |
| `iris-{ver}-windows-aarch64-setup.exe` |
| `iris-{ver}-windows-aarch64-portable.zip` |
| `iris-{ver}-macos-universal.dmg` |
| `iris-{ver}-linux-x86_64.AppImage` |
| `iris-{ver}-linux-x86_64.deb` |
| `iris-{ver}-linux-x86_64.rpm` |
| `iris-{ver}-linux-aarch64.AppImage` |
| `iris-{ver}-linux-aarch64.deb` |
| `iris-{ver}-linux-aarch64.rpm` |

Check every checksum, then every signature and its trusted comment. Each `.minisig` has the trusted comment `file:<asset file name> version:<version>`:

```sh
sha256sum -c ./*.sha256
ver=0.2.0
for f in iris-*; do
  case $f in *.sha256 | *.minisig) continue ;; esac
  comment=$(minisign -V -Q -p ../../packaging/minisign.pub -m "$f") || { echo "bad signature: $f"; continue; }
  [ "$comment" = "file:$f version:$ver" ] || echo "wrong trusted comment on $f: $comment"
done
```

`sha256sum -c` prints `OK` for each asset, and the loop prints nothing.

Check what the updater reads. For a stable release, `releases/latest` is the new tag:

```sh
gh api repos/santhreal/iris/releases/latest --jq .tag_name
```

For a prerelease, the same command prints the previous stable tag, and the new tag is in the list the `beta` channel reads:

```sh
gh api 'repos/santhreal/iris/releases?per_page=30' --jq '.[] | [.tag_name, .prerelease, .draft] | @tsv'
```

On a computer with an earlier release installed, `iris --check-update` reports the new version: with `update_channel = "stable"` for a stable release, and with `update_channel = "beta"` for a prerelease.
