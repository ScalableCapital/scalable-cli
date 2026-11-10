# Command reference

Examples and behavior notes for Scalable CLI. Run `sc <command> --help` for all
available flags. Use `--json` for scripts and integrations.

[Scalable CLI](https://github.com/ScalableCapital/scalable-cli) · [Automation](automation.md) · [Configuration](configuration.md)

## Contents

- [Identity and session](#identity-and-session)
- [Broker context](#broker-context)
- [Portfolio and market data](#portfolio-and-market-data)
- [Cash and buying power](#cash-and-buying-power)
- [Overnight savings](#overnight-savings)
- [Fixed-Term savings](#fixed-term-savings)
- [Watchlist, alerts and savings plans](#watchlist-alerts-and-savings-plans)
- [Trading](#trading)
- [Trade submission recovery](#recovering-from-a-failed-submission)

## Identity and session

```bash
sc whoami
sc logout
```

`sc whoami` also reports `session.local_read_only`; JSON output includes the
boolean at `data.session.local_read_only`.

For session limits, refresh, and revocation, see the
[automation guide](automation.md#session-lifetimes-and-refresh).

## Broker context

```bash
sc broker portfolios
sc broker context show
sc broker context select --portfolio-id <PORTFOLIO_ID>
```

`sc broker portfolios` discovers accessible broker portfolio IDs without
changing the saved context. The result reports `none_available`,
`auto_resolved`, or `selection_required`; persist an explicit choice only with
`sc broker context select --portfolio-id <PORTFOLIO_ID>`.

## Portfolio and market data

Portfolio analytics (`sc broker analytics`) is unavailable in this
version. Portfolio overview, holdings, and transaction history remain available.

```bash
sc broker overview
sc broker cash-breakdown
sc broker transactions
sc broker transactions --crypto-ticker BTC --crypto-cursor <CURSOR>
sc broker transaction details --transaction-id <TRANSACTION_ID>
sc broker holdings
sc broker order-venues
sc broker portfolio-groups
sc broker portfolio-groups --group-id <GROUP_ID>
sc broker chart --timeframe 1y
sc broker chart --portfolio-id <PORTFOLIO_ID> --timeframe ytd --json
sc broker chart --isin US0378331005 --timeframe 1m
sc broker candlesticks --isin US0378331005 --timeframe one_month
sc broker candlesticks --isin US0378331005 --timeframe one_month --quote-source eix --json
sc broker quote --isin US0378331005
sc broker watchlist
sc broker search "apple"
sc broker derivatives search --underlying US0378331005 --type knockout --strategy long
sc broker security-news --isin US0378331005 --locale en_DE
```

Holdings output keeps securities in `items` and lists crypto coin positions in
`crypto_items`, including every backing ETP ISIN. Transactions use the same
separation: broker activity is in `items`, while crypto activity and its
independent pagination cursor are in `crypto.items` and `crypto.cursor`.
Continue each stream with `--cursor` or `--crypto-cursor` while preserving the
same filters. Transaction lists and details do not include document links.
View transaction documents in the Scalable app.

`sc broker chart --timeframe <TIMEFRAME>` returns historical portfolio valuation
and monetary absolute return. It uses your saved broker context or the sole
accessible portfolio. Use `--portfolio-id` to override the portfolio, or `--isin`
for a security price chart. These two flags are mutually exclusive.

Timeframes: `1d`, `7d`, `1m`, `3m`, `6m`, `ytd`, `1y`, `max`.
Use `--json` for all points and the separate closing reference point. Portfolio
JSON includes account/portfolio resolution and a `target: "portfolio"` result;
monetary fields can be numbers or decimal strings. A series may contain only a
closing reference and zero data points. Charts contain historical data only,
with no live updates or calculated percentage returns.

`sc broker overview` reports the absolute return (`simpleAbsoluteReturn`) for
each available timeframe. It does not report a relative return percentage.

`sc broker chart` reports `broker_chart_data_unavailable` when the requested
timeframe is absent from a valid response. Try another timeframe or security;
the command does not substitute a different chart series. Malformed responses
report `broker_response_invalid`.

For cash and buying power, see [Cash and buying power](#cash-and-buying-power).

`sc broker candlesticks` returns open, high, low, and close decimal strings and
UTC interval bounds. It preserves backend values and order without generating
or correcting candles. Supported timeframes are `intraday`, `two_days`,
`one_week`, `one_month`, `three_months`, `six_months`, `year_to_date`, `one_year`,
and `max`.

`--quote-source` accepts `consolidated` (the default), `gettex`/`MUNC`,
`eix`/`SEIX`, and `vestima`/`XVES`, case-insensitively. Availability varies by
source, security, and timeframe; the command never substitutes another source.
Empty history in a present series returns `candlesticks: []`; missing or
malformed series report `broker_response_invalid`.

`sc broker holdings`, `sc broker quote`, and `sc broker watchlist` (listing)
support the same `--quote-source` options and default. `sc broker search`
always uses consolidated quotes.

`sc broker order-venues` lists the supported venues currently enabled for the
resolved portfolio. Its `venue` values (`gettex`, `xetra`, or `eix`) are valid
inputs for trade previews with `--venue`. The list does not prove that a
particular security or order is tradable; run a fresh trade preview to validate
the selected venue and order inputs.

`sc broker portfolio-groups` returns all groups, their items, group-level since-buy
performance, and ungrouped holdings. Add `--group-id <GROUP_ID>` to narrow the
result to one existing group; filtered reads keep the same JSON shape but return
an empty `ungrouped_items` list.

Manage group metadata with lifecycle commands. `update` changes only the supplied
fields; use `--clear-description` to remove an existing description.

```bash
sc broker portfolio-groups create --name "AI portfolio" --description "tracked by agent"
sc broker portfolio-groups update --group-id <GROUP_ID> --name "Long-term portfolio"
sc broker portfolio-groups update --group-id <GROUP_ID> --clear-description
sc broker portfolio-groups delete --group-id <GROUP_ID>
sc broker portfolio-groups assign --group-id <GROUP_ID> --isin US0378331005 --isin IE00B4ND3602
sc broker portfolio-groups unassign --group-id <GROUP_ID> --isin US0378331005
```

`assign` moves each supplied holding into the target group, automatically removing it
from any other group. `unassign` removes holdings from the named group and returns them
to the ungrouped holdings list. Assigning a holding already in the target group, or
unassigning one that is not in it, returns a validation error.

`sc broker derivatives search` supports derivative discovery for a known
underlying ISIN. The selected derivative ISIN can then be quoted with
`sc broker quote --isin ...`. The CLI also verifies trade buy phase-1 input
acceptance for derivative-style ISINs.

## Cash and buying power

```bash
sc broker cash-breakdown
sc broker cash-breakdown --json
```

View cash, buying power, credit, and derivatives availability for the selected
broker portfolio in one compact response.

Key fields include:

- `cash_balance`: broker cash balance.
- `buying_power`: amount available to invest, including available credit.
- `buying_power_without_credit`: amount available to invest without credit.
- `available_credit_line` and `loaned`: available credit line and borrowing.

The response also includes pending buy-order amounts, estimated taxes, and
derivatives-specific availability. Values unavailable from the backend are
returned as `null`; do not interpret them as zero.

## Overnight savings

```bash
sc overnight
sc overnight --savings-account-id <SAVINGS_ACCOUNT_ID>
sc overnight transactions --page-size 50 --type-filter interest
sc overnight transactions --savings-account-id <SAVINGS_ACCOUNT_ID> --json
```

`sc overnight` shows the dedicated overnight savings summary. It
auto-resolves the savings account when exactly one active account is accessible;
otherwise, provide `--savings-account-id`.

`sc overnight transactions` lists the deposits, withdrawals, interest, and
other cash transactions for the selected overnight account. It supports cursor
pagination, repeatable `--type-filter`, text search, and time-window filters.
Transaction documents are available in the Scalable app and are not exposed by
the CLI. Run `sc overnight transactions --help` for all available filters.

## Fixed-Term savings

```bash
sc fixed-term list
sc fixed-term list --json
sc fixed-term details --fixed-term-account-id <FIXED_TERM_ACCOUNT_ID>
sc fixed-term details --fixed-term-account-id <FIXED_TERM_ACCOUNT_ID> --json
```

List your supported personal accounts, then use a returned ID to view the deposit,
term, rates, maturity, accrued gross interest, estimated payout, auto-renewal setting,
and available trustee allocation. Amounts and rates are decimal strings in JSON;
unavailable values are `null` (shown as `<unavailable>` in text).

Supported states are `PENDING`, `ACTIVE`, and `CANCELLATION_REQUESTED`. Junior
accounts and unfinished onboarding (`CREATED` / `DOI_ACCEPTED`) are excluded,
including when an ID is supplied directly. The backend does not currently return
terminal accounts. Estimated payout is the backend estimate, not a guaranteed
amount. These commands only read data; they do not create, cancel, withdraw from,
or change auto-renewal for an account.

Unrecognized ownership or state values are reported as invalid backend responses.

## Watchlist, alerts and savings plans

```bash
sc broker watchlist add --isin US0378331005
sc broker watchlist remove --isin US0378331005
sc broker price-alerts --active-only
sc broker price-alerts add --isin US0378331005 --price 180.00
sc broker price-alerts add --ticker BTC --price 45000.00
sc broker price-alerts remove --alert-id <ALERT_ID>
sc broker savings-plans
sc broker savings-plans config --isin US0378331005
# Phase 1: preview; this does not create or update a plan.
sc broker savings-plans add --isin US0378331005 --amount 100
# Phase 2: run only after a separate affirmative client confirmation.
sc broker savings-plans add --isin US0378331005 --amount 100 --confirm <CONFIRMATION_ID>
sc broker savings-plans remove --isin US0378331005
```

Savings-plan additions and updates are intentionally two-step. Phase 1 returns
the full ex-ante cost disclosure calculated for `SEIX`, together with a
short-lived confirmation ID. Present every value in that disclosure to the
client, obtain an explicit affirmative response in a separate interaction,
then repeat the exact arguments with `--confirm <CONFIRMATION_ID>`.
Savings-plan confirmation is bound to the disclosed cost snapshot. Changes to
that snapshot require a new preview and confirmation, including numeric-format
changes that alter its checksum.

When `--day-of-month` is supplied without `--year-month`, the CLI selects the
earliest available execution month for that specific day. Supplying
`--year-month` remains an explicit override and must be available for the
selected day.

If phase 2 has an unknown outcome, do not retry it. Run
`sc broker savings-plans` for the same account and portfolio to inspect the
result; only that successful list clears the local safety gate for a later,
deliberately new preview.

## Trading

Buy and sell flows are intentionally two-step.

Run the command once to preview the order and receive a confirmation ID:

```bash
sc broker trade buy --isin US0378331005 --amount 500 --order-type market
sc broker trade buy --isin US0378331005 --shares 2 --order-type market
```

Review the preview and obtain an explicit affirmative client response in a
separate interaction. Then submit the exact same order with `--confirm` to
place it:

```bash
sc broker trade buy --isin US0378331005 --amount 500 --order-type market \
  --confirm <CONFIRMATION_ID>
sc broker trade buy --isin US0378331005 --shares 2 --order-type market \
  --confirm <CONFIRMATION_ID>
```

If phase 1 explicitly requires acknowledgement of an appropriateness warning for
the buy order, set `--acknowledge-appropriateness-warning` only after reviewing
the warning and confirming that you understand the risks and want to proceed.

Buy orders require exactly one sizing input: `--amount` or `--shares`. Buy-side
share input currently accepts whole shares only.

Buy amounts below one share at the current ask price return
`order_size_below_one_share` in JSON output with exit code `10`.

To request a specific execution venue for a trade, use `--venue`. The following
names and MICs are accepted: `gettex` or `MUNC`, `xetra` or `XETR`, and `eix`
or `SEIX`. The selected execution venue is also used for the trade preview.

The same confirmation model applies to sell orders:

```bash
sc broker trade sell --isin US0378331005 --shares 1 --order-type market
sc broker trade cancel --order-id <ORDER_ID>
```

Supported order types:

- `market`
- `limit`
- `stop`
- `stop-limit` (requires both `--stop-price` and `--limit-price`)
- `trailing-stop` (sell only, with either `--trail-amount` or `--trail-percent`)

For stop-limit orders, the stop triggers activation and the limit constrains the
execution price. Execution depends on market prices and available liquidity.
Preview either side first:

```bash
sc broker trade buy --isin US0378331005 --shares 2 --order-type stop-limit \
  --stop-price 190.00 --limit-price 191.00
sc broker trade sell --isin US0378331005 --shares 1 --order-type stop-limit \
  --stop-price 170.00 --limit-price 169.00
```

After reviewing the preview and obtaining a separate affirmative client response,
repeat its exact inputs with the confirmation ID returned by that preview:

```bash
sc broker trade buy --isin US0378331005 --shares 2 --order-type stop-limit \
  --stop-price 190.00 --limit-price 191.00 --confirm <CONFIRMATION_ID>
sc broker trade sell --isin US0378331005 --shares 1 --order-type stop-limit \
  --stop-price 170.00 --limit-price 169.00 --confirm <CONFIRMATION_ID>
```

For a trailing stop loss sell on EIX or gettex, set either a distance in EUR or a
percentage distance. `--trail-percent 5` means 5%, not 0.05%. The minimum distance
is 0.5% of the bid price. The distance must produce a positive initial stop below
the bid price, so 100% is invalid. Inputs support up to 10 fractional digits.
Do not combine a trailing distance with `--stop-price` or `--limit-price`.

```bash
sc broker trade sell --isin US0378331005 --shares 1 --order-type trailing-stop \
  --trail-amount 5 --venue eix
sc broker trade sell --isin US0378331005 --shares 1 --order-type trailing-stop \
  --trail-percent 5 --venue gettex
```

The preview shows the distance, initial stop, venue and costs. Review the preview,
then repeat its inputs with `--confirm <CONFIRMATION_ID>` after a separate
confirmation. The confirmed initial stop stays fixed during submission. If it
becomes invalid, request a new preview. Once placed, the Broker moves the stop
when the price rises. The stop triggers a market sell, whose execution price may
be below the stop price.

`sc broker transaction details --transaction-id <ORDER_ID>` shows the trail and
latest available stop with its timestamp. When a refresh is unavailable, the
output retains the initial or last-known stop and identifies its source. This
value is not a guaranteed live quote. Use the existing cancellation command to
cancel a pending trailing order.

Trade cancellation is a separate single-step command for pending orders. Use
the `order_id` returned by `sc broker trade buy` or `sc broker trade sell`; it
is the same ID shown in `sc broker transactions`.

You can configure [local trade controls](configuration.md#local-trade-controls)
to restrict CLI-based trading. These do not change backend trading permissions.

### Recovering from a failed submission

A failed command does not always mean that no order was created. A timeout or
connection failure can leave the submission outcome unknown.

Before retrying, check `sc broker transactions` or your orders in the Scalable
app for the same portfolio:

- If the order exists, do not submit it again.
- If you cannot establish the outcome, do not create a new preview or submit
  another order.
- If you have confirmed that no order was created and intentionally retry,
  repeat the exact phase-2 command with the same `--confirm` ID while it remains
  valid. This preserves the original submission's duplicate protection.

Trade confirmation IDs expire after 15 minutes. If a new preview is required,
first resolve the previous submission's outcome, then review the new preview
and obtain a separate affirmative confirmation before submitting.

If the error explicitly reports a backend rejection, treat the submission as
failed and review the reported reason before trying again.
