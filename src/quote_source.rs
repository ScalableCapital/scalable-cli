use clap::ValueEnum;
use serde_json::Value;

/// Quote sources supported by the CLI; execution venues are separate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum QuoteSource {
    #[default]
    Consolidated,
    #[value(alias = "gettex")]
    Munc,
    #[value(alias = "eix")]
    Seix,
    #[value(alias = "vestima")]
    Xves,
}

impl QuoteSource {
    pub fn as_cli_value(self) -> &'static str {
        match self {
            Self::Consolidated => "consolidated",
            Self::Munc => "munc",
            Self::Seix => "seix",
            Self::Xves => "xves",
        }
    }

    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::Consolidated => "CONSOLIDATED",
            Self::Munc => "MUNC",
            Self::Seix => "SEIX",
            Self::Xves => "XVES",
        }
    }
}

/// Check the reported quote venue before using prices.
/// Null/empty ticks retain the existing unavailable-quote behavior.
pub(crate) fn validate_quote_tick(tick: &Value) -> Result<(), &'static str> {
    let has_prices = ["midPrice", "askPrice", "bidPrice"]
        .iter()
        .any(|field| tick.get(field).is_some_and(|value| !value.is_null()));
    if has_prices
        && !matches!(
            tick.get("venue").and_then(Value::as_str),
            Some("MUNC" | "SEIX" | "XVES")
        )
    {
        return Err("quote venue is missing or unsupported");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{BrokerArgs, BrokerCommand, Cli, Commands};
    use clap::Parser;
    use serde_json::json;

    #[test]
    fn selectable_read_commands_accept_case_insensitive_sources_and_aliases() {
        for (alias, source) in [
            ("CONSOLIDATED", QuoteSource::Consolidated),
            ("gettex", QuoteSource::Munc),
            ("mUnC", QuoteSource::Munc),
            ("eix", QuoteSource::Seix),
            ("SEIX", QuoteSource::Seix),
            ("vestima", QuoteSource::Xves),
            ("XVES", QuoteSource::Xves),
        ] {
            for command in ["holdings", "watchlist", "quote", "candlesticks"] {
                let mut argv = vec!["sc", "broker", command];
                match command {
                    "quote" | "candlesticks" => argv.extend(["--isin", "US0378331005"]),
                    _ => {}
                }
                if command == "candlesticks" {
                    argv.extend(["--timeframe", "one_month"]);
                }
                let defaults = Cli::try_parse_from(&argv).unwrap();
                assert_eq!(selected_source(defaults), QuoteSource::Consolidated);
                argv.extend(["--quote-source", alias]);
                assert_eq!(selected_source(Cli::try_parse_from(argv).unwrap()), source);
            }
        }
    }

    fn selected_source(cli: Cli) -> QuoteSource {
        let Commands::Broker(BrokerArgs { command }) = cli.command else {
            panic!("broker command")
        };
        match command {
            BrokerCommand::Holdings(args) => args.quote_source.unwrap_or_default(),
            BrokerCommand::Watchlist(args) => args.quote_source.unwrap_or_default(),
            BrokerCommand::Quote(args) => args.quote_source.unwrap_or_default(),
            BrokerCommand::Candlesticks(args) => args.quote_source,
            _ => panic!("quote-bearing command"),
        }
    }

    #[test]
    fn watchlist_source_selection_is_listing_only() {
        for action in ["add", "remove"] {
            let cli = Cli::try_parse_from([
                "sc",
                "broker",
                "watchlist",
                "--quote-source",
                "eix",
                action,
                "--isin",
                "US0378331005",
            ])
            .unwrap();
            let Commands::Broker(BrokerArgs {
                command: BrokerCommand::Watchlist(args),
            }) = cli.command
            else {
                panic!("watchlist")
            };
            let error = crate::normalize_watchlist_args(args)
                .unwrap_err()
                .to_string();
            assert!(error.contains("`--quote-source` is only supported"));
        }
    }

    #[test]
    fn unsupported_sources_are_rejected_during_input_parsing() {
        for source in ["xxxx", "xetr", "unknown"] {
            assert!(
                Cli::try_parse_from([
                    "sc",
                    "broker",
                    "quote",
                    "--isin",
                    "US0378331005",
                    "--quote-source",
                    source
                ])
                .is_err()
            );
        }
    }

    #[test]
    fn price_bearing_ticks_require_a_supported_venue() {
        for field in ["midPrice", "askPrice", "bidPrice"] {
            for venue in [json!(null), json!("XXXX"), json!("XETR"), json!("UNKNOWN")] {
                let mut tick = json!({"venue": venue});
                tick[field] = json!(12345.6789);
                let error = validate_quote_tick(&tick).unwrap_err();
                assert!(!error.contains("12345"));
            }
            for venue in ["MUNC", "SEIX", "XVES"] {
                let mut tick = json!({"venue": venue});
                tick[field] = json!(100);
                assert!(validate_quote_tick(&tick).is_ok());
            }
        }
        for absent in [Value::Null, json!({}), json!({"midPrice": null})] {
            assert!(validate_quote_tick(&absent).is_ok());
        }
    }
}
