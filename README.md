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
- Work interactively in the terminal or use `--json` for scripts and agents.
- Review trades and savings-plan changes before submission with an explicit
  two-step confirmation flow.

## Install

### macOS

#### Homebrew

```bash
brew tap ScalableCapital/tap
brew trust --formula ScalableCapital/tap/scalable-cli
brew install scalable-cli
```

#### Manual install

Download the macOS PKG installer from the
[latest release](https://github.com/ScalableCapital/scalable-cli/releases)
and run it.

### Linux

Download the tar.gz archive for your architecture (`x86_64` or `aarch64`) from
the [latest release](https://github.com/ScalableCapital/scalable-cli/releases),
extract it, and move the `sc` binary to a directory on your `PATH`.

Official Linux binaries require **glibc 2.38 or newer**. Older distributions,
such as Debian 12 (`glibc 2.36`), cannot run these binaries. Official release
artifacts do not support musl-based distributions such as Alpine. Building
from source on an older distribution is a workaround.

### Provenance and verification

Official Scalable-distributed binaries are the GitHub release assets and
packages from the official Scalable Homebrew tap. Source builds, forks,
third-party packages, containers, and redistributed binaries are not
Scalable-distributed binaries. Only use binaries whose provenance you trust.

Release assets include signed checksums. See
[Verify release artifacts](public-docs/verify-releases.md) for the signing key
and complete verification commands.

```bash
sc --version
sc --help
```

### Build from source

Build the production-channel binary with a Rust toolchain:

```bash
cargo build --locked --release --features channel-prod --bin sc
target/release/sc --version
target/release/sc --help
```

The binary is written to `target/release/sc`. A self-built binary is not an
official Scalable-distributed release artifact.

## Quick start

Before you authenticate, enable Scalable CLI on the Scalable web platform
under **Profile → Security → Agentic Investing**. Then authenticate and confirm
that the CLI can access your Broker account:

```bash
sc login
sc whoami
sc capabilities --json
sc broker overview --json
```

`sc login` uses an OAuth 2.0 device-code flow. Complete login yourself in a
browser rather than via an AI agent.

For a locally enforced read-only session, use `sc login --local-read-only`.
This blocks CLI write commands locally; it does not restrict backend token
permissions. See [Local read-only mode](public-docs/configuration.md#local-read-only-mode).

### Refresh recovery

Once the rotated refresh token has been saved locally, temporary issuer
verification failures or failures saving the final verified session can be
retried. In `--json` mode these are reported as
`refresh_retryable` with exit code `30`; retry the same command. A response
whose token fails local signature or claim validation still requires `sc login`.

## Common commands

| Task | Example |
| --- | --- |
| Check identity | `sc whoami` |
| Discover broker portfolios | `sc broker portfolios` |
| Select a portfolio | `sc broker context select --portfolio-id <PORTFOLIO_ID>` |
| View portfolio overview | `sc broker overview --json` |
| View historical portfolio chart | `sc broker chart --timeframe 1y --json` |
| View cash, buying power, and credit | `sc broker cash-breakdown --json` |
| List holdings | `sc broker holdings --json` |
| List transactions | `sc broker transactions --json` |
| View a transaction | `sc broker transaction details --transaction-id <TRANSACTION_ID> --json` |
| Find a security | `sc broker search "apple" --json` |
| Get a quote | `sc broker quote --isin US0378331005 --quote-source eix --json` |
| Get OHLC candles | `sc broker candlesticks --isin US0378331005 --timeframe one_month --quote-source eix --json` |
| Discover execution venues | `sc broker order-venues` |
| View portfolio groups | `sc broker portfolio-groups` |
| List savings plans | `sc broker savings-plans` |
| View Overnight savings | `sc overnight` |
| List Fixed-Term accounts | `sc fixed-term list` |
| End the local session | `sc logout` |

Broker commands use the resolved portfolio. If selection is required, discover
its ID with `sc broker portfolios`, then select it with `sc broker context select`.

`sc broker chart` returns historical portfolio valuation and monetary absolute
return. Use `--portfolio-id` to select a portfolio or `--isin` for security price
history. See [Portfolio and market data](public-docs/commands.md#portfolio-and-market-data)
for timeframes and output details.

`cash-breakdown` distinguishes `cash_balance`, buying power with and without
credit, `available_credit_line`, and `loaned`. Unavailable values are `null`;
do not interpret them as zero. See [Cash and buying power](public-docs/commands.md#cash-and-buying-power).

See the [command reference](public-docs/commands.md) for full examples, filters,
account coverage, portfolio groups, derivatives, alerts, and savings-plan changes.
Transaction documents are available in the Scalable app and are not exposed by
the CLI. Portfolio analytics (`sc broker analytics`) is unavailable in this version.

## Trading

Buy and sell orders use two steps:

1. Preview the order and review its details and disclosures.
2. Obtain an explicit affirmative client response in a separate interaction,
   then repeat the exact arguments with the returned `--confirm` ID.

```bash
# Phase 1: preview only.
sc broker trade buy --isin US0378331005 --amount 500 --order-type market
# Phase 2: run only after separate affirmative client confirmation.
sc broker trade buy --isin US0378331005 --amount 500 --order-type market \
  --confirm <CONFIRMATION_ID>
```

Supported order types are `market`, `limit`, `stop`, and `stop-limit`.
Stop-limit orders require both `--stop-price` and `--limit-price`; see the
[trading examples](public-docs/commands.md#trading).

Trade confirmation IDs expire after 15 minutes. If the preview requires an
appropriateness-warning acknowledgement, review and acknowledge it before submission.

A failed submission can have an unknown outcome. Check transactions or orders
in the Scalable app before retrying. Do not submit another order while the
outcome is unresolved. Follow the [trade recovery procedure](public-docs/commands.md#recovering-from-a-failed-submission).

Savings-plan changes have their own [inspection and recovery procedure](public-docs/commands.md#watchlist-alerts-and-savings-plans).
Savings-plan confirmation is bound to the disclosed cost snapshot.
See [Trading](public-docs/commands.md#trading) for sell orders, sizing, venues,
order types, and cancellation.

## Automation

Use `--json` for structured output and `sc capabilities --json` to discover the
installed command surface and local trade-control state. Login requires human
interaction.

- **Session limits:** access tokens last 20 minutes and refresh automatically.
  Authenticated requests, including reads, reset a 24-hour idle timeout. A new
  interactive login is required after at most seven days from the original login.
- **Rate limits:** limits vary by operation and can change; fixed thresholds
  are not published. Rate-limited backend requests are not automatically retried.
  Respect retry guidance and check trade outcomes before retrying submissions.
- **JSON compatibility:** compatibility across releases is not guaranteed.
  Review release notes and verify the fields your integration needs before upgrading.

See the [automation guide](public-docs/automation.md) for session refresh and
revocation, rate-limit errors, JSON handling, and upgrade guidance.

## Configuration

`config.toml` is optional. Its location is:

- macOS: `~/.config/scalable-cli/config.toml`
- Linux: `$XDG_CONFIG_HOME/scalable-cli/config.toml` when `XDG_CONFIG_HOME` is
  set, otherwise `~/.config/scalable-cli/config.toml`

Sessions default to `keyring` on macOS and Linux. Signing keys default to
`secure_enclave` on macOS and `file` on Linux.

On a Linux server or container without Secret Service, use file-based session
storage by adding this to `config.toml`:

```toml
[auth]
session_backend = "file"
```

Then run `sc login` and `sc whoami`. Session secrets are stored in local files;
keep them private to the Unix account running the CLI. Prefer keyring-backed
storage where available.

See [Configuration](public-docs/configuration.md) for backend options, local
read-only mode, trade controls, PKCS#11, and dedicated Unix-user setups.

## Further reading and help

| Guide | Covers |
| --- | --- |
| [Command reference](public-docs/commands.md) | Command examples, product behavior, trading, and recovery |
| [Automation](public-docs/automation.md) | Session limits, rate limits, JSON, and upgrades |
| [Configuration](public-docs/configuration.md) | Storage, local controls, and advanced setups |
| [Verify release artifacts](public-docs/verify-releases.md) | Provenance, signed checksums, and macOS verification |

### Help

```bash
sc --help
sc broker --help
sc broker trade buy --help
```
