# Verify release artifacts

Check the origin and integrity of a downloaded Scalable CLI release.

[Scalable CLI](https://github.com/ScalableCapital/scalable-cli)

## Official binaries and source builds

Official Scalable-distributed binaries are the release assets published by
Scalable Capital on GitHub and the packages installed from the official
Scalable Homebrew tap.

If you build this repository yourself, use a fork, install from a third-party
package manager, pull a container image, or receive a redistributed binary from
someone else, that binary is not distributed by Scalable Capital. Only use
binaries whose source and provenance you trust, especially before logging in or
running broker commands.

Release assets include checksums and a minisign signature for the checksum
manifest. The examples below assume `minisign` and the GitHub CLI (`gh`) are
installed.

Each snippet below pins the current Scalable Capital minisign release signing
public key: `RWRKuuSASIzbSYpuU5gdXeTkXirJBl5+XVXLP6E60hBUUKZ5HPIGjV8b`.

## Verify a Linux tarball

```bash
tag="vX.Y.Z"
arch="x86_64" # or aarch64
repo="ScalableCapital/scalable-cli"
minisign_public_key="RWRKuuSASIzbSYpuU5gdXeTkXirJBl5+XVXLP6E60hBUUKZ5HPIGjV8b"

asset="sc-${tag}-linux-${arch}-gnu.tar.gz"
checksums="sc-${tag}-SHA256SUMS"

gh release download "${tag}" \
  --repo "${repo}" \
  --pattern "${asset}" \
  --pattern "${checksums}" \
  --pattern "${checksums}.minisig" \
  --clobber

minisign -V \
  -P "${minisign_public_key}" \
  -m "${checksums}" \
  -x "${checksums}.minisig"

set -o pipefail
grep -F "  ${asset}" "${checksums}" | sha256sum -c -
```

## Verify a macOS PKG installer

```bash
tag="vX.Y.Z"
repo="ScalableCapital/scalable-cli"
minisign_public_key="RWRKuuSASIzbSYpuU5gdXeTkXirJBl5+XVXLP6E60hBUUKZ5HPIGjV8b"

asset="sc-${tag}-macos-universal2.pkg"
checksums="sc-${tag}-SHA256SUMS"

gh release download "${tag}" \
  --repo "${repo}" \
  --pattern "${asset}" \
  --pattern "${checksums}" \
  --pattern "${checksums}.minisig" \
  --clobber

minisign -V \
  -P "${minisign_public_key}" \
  -m "${checksums}" \
  -x "${checksums}.minisig"

set -o pipefail
grep -F "  ${asset}" "${checksums}" | shasum -a 256 -c -
pkgutil --check-signature "${asset}"
spctl -a -vv --type install "${asset}"
```

## Verify the macOS Homebrew runtime ZIP

```bash
tag="vX.Y.Z"
repo="ScalableCapital/scalable-cli"
minisign_public_key="RWRKuuSASIzbSYpuU5gdXeTkXirJBl5+XVXLP6E60hBUUKZ5HPIGjV8b"

asset="sc-${tag}-macos-universal2.zip"
checksums="sc-${tag}-SHA256SUMS"

gh release download "${tag}" \
  --repo "${repo}" \
  --pattern "${asset}" \
  --pattern "${checksums}" \
  --pattern "${checksums}.minisig" \
  --clobber

minisign -V \
  -P "${minisign_public_key}" \
  -m "${checksums}" \
  -x "${checksums}.minisig"

set -o pipefail
grep -F "  ${asset}" "${checksums}" | shasum -a 256 -c -

rm -rf ./sc-homebrew-runtime
unzip -q "${asset}" -d ./sc-homebrew-runtime
codesign -dv --verbose=4 ./sc-homebrew-runtime/Sc.app
spctl -a -vv --type exec ./sc-homebrew-runtime/Sc.app
```
