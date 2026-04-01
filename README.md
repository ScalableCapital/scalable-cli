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
1. Download the tar.gz archive for your architecture (x86_64 or ARM) from the [latest release](https://github.com/ScalableCapital/scalable-cli/releases).
2. Extract the archive.
3. Move the `sc` binary to a directory on your `PATH`.

### Verification

```bash
sc --version
sc --help
```

Release assets also include checksums and Sigstore signature material for users
who want to verify integrity and provenance before installing.

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
sc broker holdings
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
sc broker savings-plans
sc broker savings-plans add --isin US0378331005 --amount 100
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
```

Supported order types:

- `market`
- `limit`
- `stop`

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

`config.toml` is optional local runtime config for storage backends. The default configuration is the most secure one per platform.

Example:

```
[auth]
session_backend = "keyring"
signing_key_backend = "secure_enclave"
```

Configuration options:

- session_backend: where the login session is stored. Supported values: `keyring`, `file`. Default is `keyring` on macOS/Linux.
- signing_key_backend: where the signing key is stored. Supported values: `file`, `secure_enclave`. Default is `secure_enclave` on macOS and `file` on Linux. `secure_enclave` is macOS-only.

Config file locations:

- macOS: ~/.config/scalable-cli/config.toml
- Linux: $XDG_CONFIG_HOME/scalable-cli/config.toml, falling back to ~/.config/scalable-cli/config.toml

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
