# Automation

Session behavior, rate limits, and JSON compatibility for scripts and agents.

[Scalable CLI](https://github.com/ScalableCapital/scalable-cli) · [Command reference](commands.md) · [Configuration](configuration.md)

## Capabilities and output

- `sc capabilities --json` exposes the supported machine-readable command surface.
- `sc capabilities --json` also exposes the parsed local trade-control state in
  `local_trade_controls`.
- Broker commands support `--json` for compact structured output.
- `sc login` remains human-oriented in the current version.

## Session lifetimes and refresh

The CLI automatically refreshes access tokens when needed. Current session
limits are:

- **Access token:** 20 minutes. Its expiry does not itself require another login.
- **Idle timeout:** 24 hours without an authenticated request. Read-only requests
  also reset this timeout.
- **Maximum session lifetime:** seven days from the original login. Refreshing
  tokens does not extend this maximum.

After the idle timeout or maximum lifetime, run `sc login` again and complete
the interactive login. Unattended scripts therefore require periodic human
reauthentication.

Disabling CLI access in your Scalable web profile terminates the session.
[Local read-only mode](configuration.md#local-read-only-mode) does not change
these limits or restrict backend token permissions.

If the rotated refresh token is already durable, temporary issuer-verification
failures or failures saving the final verified session (including keyring errors)
return `refresh_retryable` in JSON output with exit
code `30`. Retry the same command; sign in again only when the CLI reports that
login is required.

## Rate limits

Commands that call Scalable services are subject to rate limits. Limits can
differ by operation and change over time; fixed thresholds are not published.

In JSON output, `rate_limited` identifies a rate-limit error. Wait before
retrying and respect any retry delay included in the error message. The CLI
does not automatically retry rate-limited backend requests. For failed trade
submissions, follow the [recovery checks](commands.md#recovering-from-a-failed-submission)
before retrying.

## JSON output and upgrades

Use `--json` for scripts and integrations. We aim to keep the JSON structure
stable, but do not guarantee compatibility across releases. Fields may be
added, changed, or removed.

We aim to mention breaking changes in the
[release notes](https://github.com/ScalableCapital/scalable-cli/releases).
Before upgrading an automated integration, review those notes and check that
the new version returns the fields your integration needs.

Scripts should ignore additional fields, handle unavailable values, and check
the command's success or error result before processing its data. Use
`sc capabilities --json` to discover the commands supported by the installed
version.

## Submission recovery

Trade orders and savings-plan changes have different recovery rules. Follow
[trade submission recovery](commands.md#recovering-from-a-failed-submission)
for buy/sell orders. For savings-plan changes with an unknown outcome, follow
the [savings-plan inspection procedure](commands.md#watchlist-alerts-and-savings-plans)
before creating a new preview.
