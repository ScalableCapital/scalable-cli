use super::*;
use crate::cli::{BrokerArgs, BrokerCommand, Cli, Commands};
use clap::{Parser, ValueEnum};

fn response() -> Value {
    json!({"candlesticksTimeSeries": {
        "isin": "US0378331005", "currency": "EUR", "source": "CONSOLIDATED",
        "candlesticks": [{"timeFrame": "ONE_MONTH", "dataPoints": [{
            "open": "00125.9500", "high": 126.24, "low": "125.9225", "close": 126.115,
            "startTimestamp": {"time": "2026-09-09T07:30:00+02:00"},
            "endTimestamp": {"time": "2026-09-09T21:00:00.000Z"}
        }]}]
    }})
}

fn map(value: &Value) -> Result<Value> {
    map_candlesticks_response(
        "US0378331005",
        BrokerCandlesticksTimeframe::OneMonth,
        QuoteSource::Consolidated,
        serde_json::from_str(&value.to_string())
            .map_err(|_| invalid("response could not be mapped"))?,
    )
}

#[test]
fn candlesticks_cli_timeframes_parse_and_map_to_backend() {
    let expected = [
        (
            BrokerCandlesticksTimeframe::Intraday,
            "intraday",
            "INTRADAY",
        ),
        (BrokerCandlesticksTimeframe::TwoDays, "two_days", "TWO_DAYS"),
        (BrokerCandlesticksTimeframe::OneWeek, "one_week", "ONE_WEEK"),
        (
            BrokerCandlesticksTimeframe::OneMonth,
            "one_month",
            "ONE_MONTH",
        ),
        (
            BrokerCandlesticksTimeframe::ThreeMonths,
            "three_months",
            "THREE_MONTHS",
        ),
        (
            BrokerCandlesticksTimeframe::SixMonths,
            "six_months",
            "SIX_MONTHS",
        ),
        (
            BrokerCandlesticksTimeframe::YearToDate,
            "year_to_date",
            "YEAR_TO_DATE",
        ),
        (BrokerCandlesticksTimeframe::OneYear, "one_year", "ONE_YEAR"),
        (BrokerCandlesticksTimeframe::Max, "max", "MAX"),
    ];
    assert_eq!(
        BrokerCandlesticksTimeframe::value_variants().len(),
        expected.len()
    );
    for (timeframe, public, backend) in expected {
        let cli = Cli::try_parse_from([
            "sc",
            "broker",
            "candlesticks",
            "--isin",
            "US0378331005",
            "--timeframe",
            public,
            "--json",
        ])
        .unwrap();
        let Commands::Broker(BrokerArgs {
            command: BrokerCommand::Candlesticks(args),
        }) = cli.command
        else {
            panic!("expected candlesticks");
        };
        assert_eq!(args.timeframe, timeframe);
        assert_eq!(args.quote_source, QuoteSource::Consolidated);
        assert_eq!(timeframe.as_cli_value(), public);
        assert_eq!(timeframe.as_graphql(), backend);
        assert!(crate::command_requests_json_envelope(&Commands::Broker(
            BrokerArgs {
                command: BrokerCommand::Candlesticks(args),
            }
        )));
        assert_eq!(
            variables(" us0378331005 ", timeframe, QuoteSource::Consolidated).unwrap(),
            json!({"isin":"US0378331005", "timeFrames":[backend], "source":"CONSOLIDATED"})
        );
    }
}

#[test]
fn candlesticks_require_isin_and_timeframe() {
    for args in [
        vec!["sc", "broker", "candlesticks"],
        vec!["sc", "broker", "candlesticks", "--isin", "US0378331005"],
        vec![
            "sc",
            "broker",
            "candlesticks",
            "--isin",
            "US0378331005",
            "--timeframe",
            "invalid",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn candlesticks_sources_parse_request_and_project_without_substitution() {
    let expected = [
        (QuoteSource::Consolidated, "consolidated", "CONSOLIDATED"),
        (QuoteSource::Munc, "munc", "MUNC"),
        (QuoteSource::Seix, "seix", "SEIX"),
        (QuoteSource::Xves, "xves", "XVES"),
    ];
    assert_eq!(QuoteSource::value_variants().len(), expected.len());
    for (source, public, backend) in expected {
        let cli = Cli::try_parse_from([
            "sc",
            "broker",
            "candlesticks",
            "--isin",
            "US0378331005",
            "--timeframe",
            "one_month",
            "--quote-source",
            public,
        ])
        .unwrap();
        let Commands::Broker(BrokerArgs {
            command: BrokerCommand::Candlesticks(args),
        }) = cli.command
        else {
            panic!("expected candlesticks");
        };
        assert_eq!(args.quote_source, source);
        assert_eq!(source.as_cli_value(), public);
        assert_eq!(source.as_graphql(), backend);
        let variables = variables(&args.isin, args.timeframe, source).unwrap();
        assert_eq!(variables["source"], backend);
        let mut value = response();
        value["candlesticksTimeSeries"]["source"] = json!(backend);
        let result = map_candlesticks_response(
            &args.isin,
            args.timeframe,
            source,
            serde_json::from_str(&value.to_string()).unwrap(),
        )
        .unwrap();
        assert_eq!(result["source"], public);
        assert_eq!(
            result["candlesticks"],
            map(&response()).unwrap()["candlesticks"]
        );
        value["candlesticksTimeSeries"]["candlesticks"][0]["dataPoints"] = json!([]);
        assert_eq!(
            map_candlesticks_response(
                &args.isin,
                args.timeframe,
                source,
                serde_json::from_str(&value.to_string()).unwrap()
            )
            .unwrap()["candlesticks"],
            json!([])
        );
    }
    assert!(
        Cli::try_parse_from([
            "sc",
            "broker",
            "candlesticks",
            "--isin",
            "US0378331005",
            "--timeframe",
            "one_month",
            "--quote-source",
            "invalid",
        ])
        .is_err()
    );
}

#[test]
fn candlesticks_validate_isin() {
    for isin in ["", "US0378331006", "000000000000", "invalid"] {
        let error = variables(
            isin,
            BrokerCandlesticksTimeframe::Max,
            QuoteSource::Consolidated,
        )
        .unwrap_err();
        assert!(error.to_string().starts_with("Broker input invalid:"));
    }
}

#[test]
fn candlesticks_query_is_read_only_and_parameterized() {
    graphql_parser::query::parse_query::<String>(QUERY).unwrap();
    assert!(QUERY.contains("$source: MarketDataSource!"));
    assert!(QUERY.contains("source: $source"));
    for field in [
        "open",
        "high",
        "low",
        "close",
        "startTimestamp",
        "endTimestamp",
    ] {
        assert!(QUERY.contains(field));
    }
    crate::graphql::enforce_graphql_access_policy(
        QUERY,
        Some("BrokerCandlesticks"),
        crate::graphql::GraphqlAccessContext::with_session_mode(Some(
            crate::session::SessionMode::LocalReadOnly,
        )),
    )
    .unwrap();
}

#[test]
fn candlesticks_map_decimal_strings_and_utc_intervals() {
    assert_eq!(
        map(&response()).unwrap(),
        json!({
            "isin":"US0378331005", "timeframe":"one_month", "source":"consolidated", "currency":"EUR",
            "candlesticks":[{"open":"125.95", "high":"126.24", "low":"125.9225", "close":"126.115",
                "start_time_utc":"2026-09-09T05:30:00.000Z", "end_time_utc":"2026-09-09T21:00:00.000Z"}]
        })
    );
}

#[test]
fn candlesticks_allow_empty_history_for_a_present_series() {
    let mut value = response();
    value["candlesticksTimeSeries"]["candlesticks"][0]["dataPoints"] = json!([]);
    assert_eq!(map(&value).unwrap()["candlesticks"], json!([]));
}

#[test]
fn candlesticks_reject_missing_duplicate_or_wrong_series() {
    for series in [
        json!([]),
        json!([{"timeFrame":"ONE_YEAR", "dataPoints":[]}]),
        json!([{"timeFrame":"ONE_MONTH", "dataPoints":[]}, {"timeFrame":"ONE_MONTH", "dataPoints":[]}]),
    ] {
        let mut value = response();
        value["candlesticksTimeSeries"]["candlesticks"] = series;
        assert!(map(&value).is_err());
    }
}

#[test]
fn candlesticks_reject_bad_currency_isin_and_shape() {
    for (field, bad) in [
        ("currency", json!("")),
        ("currency", json!("EURO")),
        ("currency", json!("123")),
        ("isin", json!("IE00B4L5Y983")),
        ("source", json!("SEIX")),
        ("source", Value::Null),
        ("candlesticks", Value::Null),
    ] {
        let mut value = response();
        value["candlesticksTimeSeries"][field] = bad;
        let error = map(&value).unwrap_err();
        assert_eq!(
            crate::machine::classify_error(&error).code,
            "broker_response_invalid"
        );
    }
    assert!(map(&json!({})).is_err());
}

#[test]
fn candlesticks_reject_invalid_prices_and_timestamps() {
    for bad in [
        Value::Null,
        json!(true),
        json!(""),
        json!("NA"),
        json!("-"),
        json!("NaN"),
        json!("1e3"),
        json!("+1"),
        json!("."),
        json!("1.2.3"),
    ] {
        let mut value = response();
        value["candlesticksTimeSeries"]["candlesticks"][0]["dataPoints"][0]["open"] = bad;
        assert!(map(&value).is_err());
    }
    for bad in ["invalid", "2026-09-10T00:00:00Z"] {
        let mut value = response();
        value["candlesticksTimeSeries"]["candlesticks"][0]["dataPoints"][0]["startTimestamp"]["time"] =
            json!(bad);
        assert!(map(&value).is_err());
    }
    for bad in [json!("invalid"), Value::Null, json!(42)] {
        let mut value = response();
        value["candlesticksTimeSeries"]["candlesticks"][0]["dataPoints"][0]["endTimestamp"]["time"] =
            bad;
        assert!(map(&value).is_err());
    }
}

#[test]
fn candlesticks_prices_preserve_precision_and_expand_numeric_exponents() {
    for (value, expected) in [
        (json!("000.00000000000000012300"), "0.000000000000000123"),
        (json!("-.5000"), "-0.5"),
        (json!(" -0012.3400 "), "-12.34"),
        (json!("-0.000"), "0"),
        (json!("12."), "12"),
        (json!(1e-10), "0.0000000001"),
        (json!(-1e-10), "-0.0000000001"),
        (json!(1e20), "100000000000000000000"),
    ] {
        assert_eq!(
            price(&RawValue::from_string(value.to_string()).unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn candlesticks_preserve_high_precision_numeric_wire_prices() {
    let raw = r#"{"candlesticksTimeSeries":{"isin":"US0378331005","currency":"EUR","source":"CONSOLIDATED","candlesticks":[{"timeFrame":"ONE_MONTH","dataPoints":[{"open":9007199254740993.1234567890123456789,"high":9007199254740994,"low":0.000000000000000123,"close":1e-10,"startTimestamp":{"time":"2026-09-09T05:30:00Z"},"endTimestamp":{"time":"2026-09-09T21:00:00Z"}}]}]}}"#;
    let mapped = map_candlesticks_response(
        "US0378331005",
        BrokerCandlesticksTimeframe::OneMonth,
        QuoteSource::Consolidated,
        serde_json::from_str(raw).unwrap(),
    )
    .unwrap();
    assert_eq!(
        mapped["candlesticks"][0]["open"],
        "9007199254740993.1234567890123456789"
    );
    assert_eq!(mapped["candlesticks"][0]["low"], "0.000000000000000123");
    assert_eq!(mapped["candlesticks"][0]["close"], "0.0000000001");
    let excessive = RawValue::from_string("1e100000".to_string()).unwrap();
    assert!(price(&excessive).is_err());
}

#[test]
fn candlesticks_raw_prices_expand_exactly_and_reject_oversized_values() {
    for (raw, expected) in [
        ("1.2e-1", "0.12"),
        ("-1.2E+2", "-120"),
        ("1e-20", "0.00000000000000000001"),
        ("-0e-20", "0"),
        ("0.12000000000000000001", "0.12000000000000000001"),
    ] {
        let value = RawValue::from_string(raw.to_string()).unwrap();
        assert_eq!(price(&value).unwrap(), expected);
    }
    for raw in [
        "null",
        "true",
        "[]",
        "{}",
        "1e4097",
        "1e-4097",
        "1e2147483648",
    ] {
        let value = RawValue::from_string(raw.to_string()).unwrap();
        assert!(price(&value).is_err(), "{raw}");
    }
    let oversized = RawValue::from_string(format!("1{}", "0".repeat(4096))).unwrap();
    assert!(price(&oversized).is_err());
}

#[test]
fn candlesticks_preserve_backend_candle_order() {
    let mut value = response();
    let first = value["candlesticksTimeSeries"]["candlesticks"][0]["dataPoints"][0].clone();
    let mut second = first.clone();
    second["close"] = json!(127);
    value["candlesticksTimeSeries"]["candlesticks"][0]["dataPoints"] = json!([first, second]);
    let mapped = map(&value).unwrap();
    assert_eq!(mapped["candlesticks"][0]["close"], "126.115");
    assert_eq!(mapped["candlesticks"][1]["close"], "127");
}
