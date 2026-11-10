#[test]
fn trailing_details_refresh_and_fallback_keep_price_timestamp_pairs() {
    for response in [
        json!({
            "data": {
                "account": {
                    "brokerPortfolio": {
                        "latestStopPrice": {
                            "stopPrice": 96.25,
                            "latestStopPriceTimestamp": {
                                "time": "2026-10-30T12:00:00Z",
                                "epochSecond": 1793361600_u64,
                                "epochMillisecond": 1793361600000_u64,
                            },
                        },
                    },
                },
            },
        }),
        json!({"data": {"account": {"brokerPortfolio": {"latestStopPrice": null}}}}),
        json!({"errors": [{"message": "Stop refresh unavailable"}]}),
        json!({
            "data": {
                "account": {
                    "brokerPortfolio": {
                        "latestStopPrice": {
                            "stopPrice": 97.0,
                            "latestStopPriceTimestamp": {
                                "time": "1970-01-01T00:00:01Z",
                                "epochSecond": 1,
                                "epochMillisecond": 1000,
                            },
                        },
                    },
                },
            },
        }),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        ensure_runtime_dpop_key(&config);
        let mut manager = SessionManager::new(&config).unwrap();
        manager
            .save_active(&sample_stored_session(crate::channel::current_env()))
            .unwrap();
        let details = server
            .mock("POST", "/")
            .match_body(Matcher::PartialJson(
                json!({"operation_name": "BrokerTransactionDetails"}),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "account": {
                            "brokerPortfolio": {
                                "transactionDetails": {
                                    "__typename": "BrokerSecurityTransaction",
                                    "id": "trailing-order",
                                    "type": "SECURITY_TRANSACTION",
                                    "currency": "EUR",
                                    "isPending": true,
                                    "transactionReference": "reference-1",
                                    "stopPrice": 95.0,
                                    "trailingStopInfo": {
                                        "trailType": "PERCENTAGE",
                                        "trailOffset": 5,
                                        "latestStopPriceTimestamp": {
                                            "time": "2026-10-29T12:00:00Z",
                                            "epochSecond": 1793275200_u64,
                                            "epochMillisecond": 1793275200000_u64,
                                        },
                                    },
                                },
                            },
                        },
                    },
                })
                .to_string(),
            )
            .expect(1)
            .create();
        let refresh = server
            .mock("POST", "/")
            .match_body(Matcher::PartialJson(json!({
                "operation_name": "latestStopPrice",
                "variables": {
                    "personId": "person-1",
                    "portfolioId": "portfolio-1",
                    "transactionReference": "reference-1",
                },
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response.to_string())
            .expect(1)
            .create();
        let payload = execute_broker_transaction_details(
            BrokerTransactionDetailsArgs {
                transaction_id: "trailing-order".into(),
                portfolio_id: Some("portfolio-1".into()),
                json: true,
            },
            &config,
            &mut manager,
        )
        .unwrap();
        let trail = &payload["result"]["security_trade"]["trailing_stop_info"];
        if response.pointer("/data/account/brokerPortfolio/latestStopPrice/stopPrice")
            == Some(&json!(96.25))
        {
            assert_eq!(trail["stop_price"], 96.25);
            assert_eq!(trail["latest_stop_price_timestamp"], "2026-10-30T12:00:00Z");
            assert_eq!(trail["refresh_status"], "refreshed");
        } else {
            assert_eq!(trail["stop_price"], 95.0);
            assert_eq!(trail["latest_stop_price_timestamp"], "2026-10-29T12:00:00Z");
            assert_eq!(trail["refresh_status"], "unavailable");
            assert_eq!(trail["stop_price_source"], "last_known");
        }
        details.assert();
        refresh.assert();
    }
}

#[test]
fn trailing_stop_refresh_is_limited_to_stale_pending_orders_with_a_reference() {
    let mut projected = json!({
        "is_pending": true,
        "transaction_reference": "ref",
        "security_trade": {
            "trailing_stop_info": {"latest_stop_price_epoch_millisecond": 1000},
        },
    });
    assert!(!trailing_stop_needs_refresh(&projected, 31_000));
    assert!(trailing_stop_needs_refresh(&projected, 31_001));
    projected["is_pending"] = json!(false);
    assert!(!trailing_stop_needs_refresh(&projected, 31_001));
    projected["is_pending"] = json!(true);
    projected["transaction_reference"] = json!("");
    assert!(!trailing_stop_needs_refresh(&projected, 31_001));
    projected["transaction_reference"] = json!("ref");
    projected["security_trade"]["trailing_stop_info"]["latest_stop_price_epoch_millisecond"] =
        Value::Null;
    assert!(!trailing_stop_needs_refresh(&projected, 31_001));
}
