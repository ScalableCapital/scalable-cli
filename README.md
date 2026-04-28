<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/sc-white.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/sc-woodsmoke.svg">
    <img src="docs/assets/sc-woodsmoke.svg" alt="Scalable CLI" width="80">
  </picture>
</p>

<br />
<br />

<h1 align="center">Scalable CLI</h1>

<p align="center"><strong>The official, agent-ready command line for the Scalable Broker.</strong></p>

<p align="center">Built for developers, local automation, and AI agents that need deterministic commands, structured JSON, and explicit confirmation for sensitive actions.</p>

<br />

<p align="center">
  <a href="https://github.com/ScalableCapital/scalable-cli/releases">Releases</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#common-commands">Common commands</a> ·
  <a href="#help">Help</a>
</p>

<br />

Scalable CLI is a standalone CLI on top of the Scalable API. It is the local execution layer for Scalable Broker when you want something sturdier than
Selenium, browser scraping, or brittle UI scripts.

## Why use `sc`

- Use supported broker commands instead of browser automation or screen scraping.
- Work interactively in the terminal or use `--json` for local scripts and agent workflows.
- Review trades before submission with an explicit two-step confirmation flow.

## Install

### macOS

#### Option 1: Homebrew
1. Tap the repository:
   ```bash
   brew tap ScalableCapital/tap
   ```
2. Install the CLI:
   ```bash
   brew install scalable-cli
   ```

#### Option 2: Manual install
1. Download the macOS PKG installer from the [latest release](https://github.com/ScalableCapital/scalable-cli/releases).
2. Run the installer.

### Linux

#### Manual install
1. Download the tar.gz archive for your architecture (`x86_64` or `aarch64`) from the [latest release](https://github.com/ScalableCapital/scalable-cli/releases).
2. Extract the archive.
3. Move the `sc` binary to a directory on your `PATH`.

### Official binaries and source builds

Official Scalable-distributed binaries are the release assets published by
Scalable Capital on GitHub and the packages installed from the official
Scalable Homebrew tap.

If you build this repository yourself, use a fork, install from a third-party
package manager, pull a container image, or receive a redistributed binary from
someone else, that binary is not distributed by Scalable Capital. Only use
binaries whose source and provenance you trust, especially before logging in or
running broker commands.

### Basic verification

```bash
sc --version
sc --help
```

## Quick start

Scalable CLI is currently in beta and we do not offer active support.

Clients need to be allowlisted before they can log in.

Generate your installation code:

```bash
sc installation-code
```

`sc installation-code` works without login.

Then, send us an email to [cli.beta@scalable.capital](mailto:cli.beta@scalable.capital) from the email address used for your Scalable account.
Use the subject `Scalable CLI Allowlisting` and include the installation code in
the message body.

After you have been allowlisted, authenticate and confirm the CLI can access
your Scalable Broker account:

```bash
sc login
sc whoami
sc capabilities --json
sc broker overview --json
```

`sc login` uses an OAuth 2.0 device-code flow.
For security and reliability, complete login yourself rather than via an AI agent.

## Release artifact provenance

Release assets include checksums and a minisign signature for the checksum
manifest. The examples below assume `minisign` and the GitHub CLI (`gh`) are
installed.

Each snippet below pins the current Scalable Capital minisign release signing
public key: `RWRKuuSASIzbSYpuU5gdXeTkXirJBl5+XVXLP6E60hBUUKZ5HPIGjV8b`.

### Verify a Linux tarball

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

### Verify a macOS PKG installer

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

### Verify the macOS Homebrew runtime ZIP

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

## Common commands

### Identity and session

```bash
sc whoami
sc logout
```

### Portfolio and market data

```bash
sc broker overview
sc broker analytics
sc broker transactions
sc broker transaction details --transaction-id <TRANSACTION_ID>
sc broker holdings
sc broker quote --isin US0378331005
sc broker watchlist
sc broker search "apple"
sc broker security-news --isin US0378331005 --locale en_DE
```

### Watchlist, alerts and savings plans

```bash
sc broker watchlist add --isin US0378331005
sc broker watchlist remove --isin US0378331005
sc broker price-alerts --active-only
sc broker price-alerts add --isin US0378331005 --price 180.00
sc broker price-alerts add --ticker BTC --price 45000.00
sc broker price-alerts remove --alert-id <ALERT_ID>
sc broker savings-plans
sc broker savings-plans add --isin US0378331005 --amount 100
sc broker savings-plans remove --isin US0378331005
```

### Broker context

```bash
sc broker context show
sc broker context select --portfolio-id <PORTFOLIO_ID>
```

## Trading

Buy and sell flows are intentionally two-step.

Run the command once to preview the order and receive a confirmation ID:

```bash
sc broker trade buy --isin US0378331005 --amount 500 --order-type market
```

Submit the exact same order with `--confirm` to place it:

```bash
sc broker trade buy --isin US0378331005 --amount 500 --order-type market \
  --confirm <CONFIRMATION_ID>
```

If phase 1 marks the instrument as not suitable, add `--accept-unsuitable` to the phase 2 command.

The same confirmation model applies to sell orders:

```bash
sc broker trade sell --isin US0378331005 --shares 1 --order-type market
sc broker trade cancel --order-id <ORDER_ID>
```

Supported order types:

- `market`
- `limit`
- `stop`

Trade cancellation is a separate single-step command for pending orders. Use
the `order_id` returned by `sc broker trade buy` or `sc broker trade sell`; it
is the same ID shown in `sc broker transactions`.

## Automation

- `sc capabilities --json` exposes the supported machine-readable command surface.
- Broker commands support `--json` for compact structured output.
- `sc login` remains human-oriented in the current version.

## Help

```bash
sc --help
sc broker --help
sc broker trade buy --help
```

## Configuration

`config.toml` is optional local runtime config for storage backends. The default
configuration is platform-specific.

Example:

```toml
[auth]
session_backend = "keyring"
signing_key_backend = "secure_enclave"
```

Configuration options:

- session_backend: where the login session is stored. Supported values: `keyring`, `file`. Default is `keyring` on macOS/Linux.
- signing_key_backend: where the authentication signing key is stored. Supported values: `file`, `secure_enclave`, `pkcs11`. Default is `secure_enclave` on macOS and `file` on Linux. `secure_enclave` is macOS-only. `pkcs11` is Linux-only and opt-in.

### Linux PKCS#11 signing key

PKCS#11 lets the CLI use an existing hardware token, smartcard, or HSM-backed
key for its authentication signing key. The CLI does not create, import, rotate,
delete, export, or store PKCS#11 private keys.

```toml
[auth]
signing_key_backend = "pkcs11"

[auth.pkcs11]
module_path = "/usr/lib/x86_64-linux-gnu/opensc-pkcs11.so"
key_uri = "pkcs11:token=YubiKey%20PIV;id=%01"
```

PKCS#11 options:

- module_path: required path to the PKCS#11 provider library.
- key_uri: required PKCS#11 URI for the existing key.

The URI must start with `pkcs11:` and identify exactly one EC P-256 private
key. Use `id=` or `object=` to identify the key; add `token=`, `serial=`,
`manufacturer=`, or `model=` if needed to make token selection unambiguous.
Percent-encode binary IDs and spaces.

If the token requires login, use either the token/provider protected
authentication path or set `SC_PKCS11_PIN` in the environment. The CLI does not
prompt for or store the PIN.

PKCS#11 failures do not fall back to `file`. Switching signing backends changes
the authentication key identity, so run `sc login` again after changing this
setting.

Config file locations:

- macOS: ~/.config/scalable-cli/config.toml
- Linux: $XDG_CONFIG_HOME/scalable-cli/config.toml, falling back to ~/.config/scalable-cli/config.toml

## Build from source

Building from source is useful for inspection, local development, and advanced
users. A locally built binary is self-built; it is not an official
Scalable-distributed release artifact unless it was built and published by
Scalable Capital through the official release process above.

Build the production-channel binary:

```bash
cargo build --locked --release --features channel-prod --bin sc
```

The binary is written to:

```bash
target/release/sc
```

Verify the build:

```bash
target/release/sc --version
target/release/sc --help
```

## Run as a dedicated Unix user

If another local service needs to invoke `sc`, it can be useful to run the CLI as
a separate Unix user with a narrowly scoped `sudoers` rule.

This lets the application use the CLI without gaining direct access to files
created by the CLI, such as session or configuration data owned by
`scalable-cli-user`. That separation is useful for limiting data exposure and
reducing the blast radius if the calling application account is compromised.

### Create the user

Create the dedicated account:

```bash
sudo useradd -m -s /bin/bash scalable-cli-user
```

If the user should not be able to log in interactively, use a non-login shell
instead:

```bash
sudo useradd -m -s /usr/sbin/nologin scalable-cli-user
```

Verify that the user exists:

```bash
id scalable-cli-user
```

### Configure `sudoers`

Assume the main application runs as Unix user `app-user` and should be allowed
to execute exactly the `sc` CLI as `scalable-cli-user`.

Create a dedicated `sudoers` file such as `/etc/sudoers.d/app-cli` with:

```sudoers
app-user ALL=(scalable-cli-user) NOPASSWD: /usr/local/bin/sc
```

This means:

- `app-user` may use `sudo` without a password.
- The command runs as `scalable-cli-user`.
- Only `/usr/local/bin/sc` is allowed.

Set the required permissions on the file:

```bash
sudo chmod 440 /etc/sudoers.d/app-cli
```

Validate the `sudoers` configuration:

```bash
sudo visudo -cf /etc/sudoers.d/app-cli
```

### Result

With that setup, `app-user` can run:

```bash
sudo -n -u scalable-cli-user /usr/local/bin/sc
```

The same rule does not allow arbitrary commands to run as
`scalable-cli-user`.

In practice, this means `app-user` can trigger `sc`, but does not automatically
get read access to files owned by `scalable-cli-user`. The CLI remains usable
while its local files stay isolated behind normal Unix file ownership.
