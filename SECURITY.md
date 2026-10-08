# Security Policy

## Supported Versions

Security fixes ship in a new release. Only the latest release receives them; an earlier release is not patched.

| Version | Supported |
| --- | --- |
| Latest release on [GitHub Releases](https://github.com/santhreal/iris/releases/latest) | Yes |
| Any earlier release or prerelease | No |

Update with `iris --update`, the **Install iris** item of the tray menu, or the Updates pane of the settings window ([Updates](docs/updates.md)).

## Reporting a Vulnerability

Report a vulnerability privately through GitHub private vulnerability reporting: open [Report a vulnerability](https://github.com/santhreal/iris/security/advisories/new) on the repository's Security tab. Do not open a public issue, pull request, or discussion for it.

Include:

- the iris version (`iris --version`);
- the operating system, its version, and the architecture (`x86_64` or `aarch64`);
- how iris was installed: Windows installer, Windows portable zip, macOS disk image, AppImage, deb, rpm, or a build from source;
- the steps that reproduce the issue, and what an attacker gains from it.

The report and its discussion are visible to you and the repository maintainers only. The advisory is published with the release that holds the fix.

## Verifying Release Signatures

Every release asset has two files beside it on the release page: `<asset>.sha256`, its SHA-256 checksum in `sha256sum` format, and `<asset>.minisig`, its [minisign](https://jedisct1.github.io/minisign/) signature. The signing key's public half is [`packaging/minisign.pub`](packaging/minisign.pub), key id `EC8B4C1097043E88`:

```
RWSIPgSXEEyL7E0nxtQR83kxxdTJGm0GcQ1/J/EOUgG+cCDn9RJjIYox
```

Download the asset, its `.sha256` and `.minisig` files, and `minisign.pub` into one directory, then check the checksum and the signature:

```sh
sha256sum -c iris-0.2.0-linux-x86_64.deb.sha256
minisign -Vm iris-0.2.0-linux-x86_64.deb -p minisign.pub
```

`minisign` prints `Signature and comment signature verified` and the trusted comment `file:<asset file name> version:<version>`. Use the asset only when the file name and the version in the trusted comment are the ones you downloaded. A signature that does not verify, or a trusted comment that differs, means the file is not the one the release published.

[Installation](docs/install.md#verifying-a-download) lists the commands for macOS and Windows. `iris --update`, the tray's **Install iris** item, and the settings window's install run the same two checks before they stop the running iris, and install nothing when either check fails.
