#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrailingFlowOutcome {
    Submit,
    ChangedDistance,
    TamperedStop,
    TamperedChecksum,
    Expired,
    BidBelowStop,
}

fn assert_trailing_flow(percent: bool, outcome: TrailingFlowOutcome) {
    let tmp = tempfile::tempdir().unwrap();
    let mut server = Server::new();
    let _channel_guard = TestChannelGuard::for_server(&server);
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let config = sample_config();
    ensure_runtime_dpop_key(&config);
    let mut session_manager = crate::session::SessionManager::new(&config).unwrap();
    session_manager
        .save_active(&sample_stored_session(crate::channel::current_env()))
        .unwrap();
    let flag = if percent {
        "--trail-percent"
    } else {
        "--trail-amount"
    };
    let distance = if percent { "5.1234567891" } else { "5" };
    let expected_stop: f64 = if percent {
        100.0 - (5.1234567891 / 100.0) * 100.0
    } else {
        95.0
    };
    let expected_volume = round_estimated_order_volume_for_ex_ante(expected_stop * 2.0);
    let command = format!(
        "sc broker trade sell --isin DE0007100000 --shares 2 --order-type trailing-stop {flag} {distance} --venue gettex --json"
    );
    let resolve = json!({
        "data": {
            "account": {
                "id": "person-1",
                "brokerPortfolios": [{"id": "portfolio-1"}],
            },
        },
    })
    .to_string();
    let no_placement = server
        .mock("POST", "/")
        .match_body(Matcher::Regex("placeOrder".to_string()))
        .expect(0)
        .with_status(500)
        .create();
    let preview_mocks = [
        mock_trade_read(
            &mut server,
            "ResolveBrokerIds",
            json!({"id": "person-1"}),
            resolve.clone(),
        ),
        mock_trade_read(
            &mut server,
            "getTradingTradability",
            json!({"isin": "DE0007100000"}),
            trade_preparation_tradability_response("MUNC"),
        ),
        mock_trade_read(
            &mut server,
            "getSecurityTick",
            json!({"source": "MUNC"}),
            trade_preparation_tick_response(101.0),
        ),
        mock_trade_read(
            &mut server,
            "getSingleTradeExAnteCost",
            json!({"side": "SELL", "venue": "MUNC", "estimatedOrderVolume": expected_volume}),
            trade_preparation_ex_ante_response(),
        ),
    ];
    let preview = execute_test_trade_command(&command, &config, &mut session_manager).unwrap();
    assert_eq!(preview["result"]["order_submission"]["submitted"], false);
    assert_eq!(
        preview["result"]["intent"]["stop_price"],
        expected_stop.to_string()
    );
    assert_eq!(
        preview["result"]["intent"]["trailing_stop"]["trail_offset"],
        distance
    );
    assert_eq!(
        preview["result"]["intent"]["trailing_stop"]["unit"],
        if percent { "%" } else { "EUR" }
    );
    assert!(
        preview["presentation"]["required_leaf_paths"]
            .as_array()
            .unwrap()
            .contains(&json!("/result/intent/trailing_stop/execution"))
    );
    assert!(
        render_trade_sell_text(&preview)
            .iter()
            .any(|line| line.starts_with(&format!("trail_distance: {distance} ")))
    );
    for mock in preview_mocks {
        mock.assert();
        mock.remove();
    }
    no_placement.assert();
    let id = preview["confirmation"]["id"].as_str().unwrap();
    let mut stored = load_confirmation(id).unwrap().unwrap();
    assert_eq!(
        stored.phase1_input.initial_stop_price.as_deref(),
        Some(expected_stop.to_string().as_str())
    );
    let mut replay = preview["confirmation"]["phase_2_command_template_json"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(replay.contains(&format!("{flag} {distance}")));
    match outcome {
        TrailingFlowOutcome::ChangedDistance => {
            replay = replay.replace(&format!("{flag} {distance}"), &format!("{flag} 6"))
        }
        TrailingFlowOutcome::TamperedStop => {
            stored.phase1_input.initial_stop_price = Some("94".to_string());
            upsert_confirmation(stored.clone(), current_epoch_seconds()).unwrap();
        }
        TrailingFlowOutcome::TamperedChecksum => {
            stored.phase1_input.initial_stop_price = Some("94".to_string());
            let mut checksum = stored.snapshot_payload.clone();
            checksum["intent"]["stop_price"] = json!("94");
            stored.intent_checksum = checksum_for_payload(&checksum);
            upsert_confirmation(stored.clone(), current_epoch_seconds()).unwrap();
        }
        TrailingFlowOutcome::Expired => {
            stored.expires_at_epoch = current_epoch_seconds() - 1;
            upsert_confirmation(stored.clone(), current_epoch_seconds()).unwrap();
        }
        _ => {}
    }
    let before_network = matches!(
        outcome,
        TrailingFlowOutcome::ChangedDistance
            | TrailingFlowOutcome::TamperedChecksum
            | TrailingFlowOutcome::Expired
    );
    let mut confirm_mocks = Vec::new();
    if !before_network {
        confirm_mocks.extend([
            mock_trade_read(
                &mut server,
                "ResolveBrokerIds",
                json!({"id": "person-1"}),
                resolve,
            ),
            mock_trade_read(
                &mut server,
                "getTradingTradability",
                json!({"isin": "DE0007100000"}),
                trade_preparation_tradability_response("MUNC"),
            ),
            mock_trade_read(
                &mut server,
                "getSecurityTick",
                json!({"source": "MUNC"}),
                trade_preparation_tick_response(if outcome == TrailingFlowOutcome::BidBelowStop {
                    94.0
                } else {
                    111.0
                }),
            ),
        ]);
        if outcome != TrailingFlowOutcome::BidBelowStop {
            confirm_mocks.push(mock_trade_read(
                &mut server,
                "getSingleTradeExAnteCost",
                json!({
                    "venue": "MUNC",
                    "estimatedOrderVolume": if outcome == TrailingFlowOutcome::TamperedStop {
                        188.0
                    } else {
                        expected_volume
                    },
                }),
                trade_preparation_ex_ante_response(),
            ));
        }
    }
    let placement = server
        .mock("POST", "/")
        .match_header("x-sc-order-origin", "CLI")
        .match_body(graphql_variables_matcher(
            "placeOrder",
            json!({
                "portfolioId": "portfolio-1",
                "isBuy": false,
                "input": {
                    "isin": "DE0007100000",
                    "side": "SELL",
                    "numberOfShares": 2.0,
                    "currency": "EUR",
                    "venue": "MUNC",
                    "stopPrice": expected_stop,
                    "trailingStopData": {
                        "trailType": if percent { "PERCENTAGE" } else { "AMOUNT" },
                        "trailOffset": distance.parse::<f64>().unwrap(),
                    },
                },
            }),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "data": {
                    "placeOrder": {
                        "brokerPortfolio": {"id": "portfolio-1"},
                        "orderData": {"orderId": "trailing-order", "isMarketable": false},
                    },
                },
            })
            .to_string(),
        )
        .expect(usize::from(outcome == TrailingFlowOutcome::Submit))
        .create();
    let confirmed = execute_test_trade_command(&replay, &config, &mut session_manager);
    if outcome == TrailingFlowOutcome::Submit {
        let confirmed = confirmed.unwrap();
        assert_eq!(
            confirmed["result"]["intent"]["stop_price"],
            expected_stop.to_string()
        );
        assert_eq!(
            confirmed["result"]["order_submission"]["order_id"],
            "trailing-order"
        );
    } else {
        let error = confirmed.unwrap_err().to_string();
        assert!(
            error.contains(if outcome == TrailingFlowOutcome::Expired {
                "CONFIRMATION_EXPIRED"
            } else {
                "CONFIRMATION_FIELDS_MISMATCH"
            }),
            "{outcome:?}: {error}"
        );
        assert!(
            load_confirmation(id)
                .unwrap()
                .unwrap()
                .consumed_at_epoch
                .is_none()
        );
    }
    for mock in confirm_mocks {
        mock.assert();
    }
    placement.assert();
    no_placement.assert();
}

#[test]
fn trailing_stop_previews_and_submits_both_distance_modes_with_the_approved_stop() {
    for percent in [false, true] {
        assert_trailing_flow(percent, TrailingFlowOutcome::Submit);
    }
}

#[test]
fn trailing_stop_confirmation_rejects_changed_tampered_expired_and_triggered_instructions() {
    for outcome in [
        TrailingFlowOutcome::ChangedDistance,
        TrailingFlowOutcome::TamperedStop,
        TrailingFlowOutcome::TamperedChecksum,
        TrailingFlowOutcome::Expired,
        TrailingFlowOutcome::BidBelowStop,
    ] {
        assert_trailing_flow(true, outcome);
    }
}

#[test]
fn trailing_stop_input_rejects_unsupported_combinations_before_network() {
    use clap::Parser;
    for suffix in [
        "",
        "--trail-percent 5 --stop-price 90",
        "--trail-percent 5 --limit-price 90",
    ] {
        let command = format!(
            "sc broker trade sell --isin DE0007100000 --shares 2 --order-type trailing-stop {suffix}"
        );
        let cli = crate::cli::Cli::try_parse_from(command.split_whitespace()).unwrap();
        let crate::cli::Commands::Broker(broker) = cli.command else {
            panic!()
        };
        let crate::cli::BrokerCommand::Trade(trade) = broker.command else {
            panic!()
        };
        let crate::cli::BrokerTradeCommand::Sell(args) = trade.command else {
            panic!()
        };
        assert!(parse_phase1_intent_sell(&args).is_err());
    }
}

#[test]
fn trailing_preview_rejects_xetra_and_missing_bid_without_placement() {
    for selected_venue in ["XETR", "MUNC"] {
        let tmp = tempfile::tempdir().unwrap();
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        ensure_runtime_dpop_key(&config);
        let mut manager = crate::session::SessionManager::new(&config).unwrap();
        manager
            .save_active(&sample_stored_session(crate::channel::current_env()))
            .unwrap();
        let resolve = mock_trade_read(
            &mut server,
            "ResolveBrokerIds",
            json!({"id": "person-1"}),
            json!({
                "data": {
                    "account": {
                        "id": "person-1",
                        "brokerPortfolios": [{"id": "portfolio-1"}],
                    },
                },
            })
            .to_string(),
        );
        let tradability = mock_trade_read(
            &mut server,
            "getTradingTradability",
            json!({"isin": "DE0007100000"}),
            trade_preparation_tradability_response(selected_venue),
        );
        let mut tick: Value =
            serde_json::from_str(&trade_preparation_tick_response(101.0)).unwrap();
        tick["data"]["account"]["brokerPortfolio"]["security"]["quoteTick"]["bidPrice"] =
            Value::Null;
        let quote = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("getSecurityTick".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(tick.to_string())
            .expect(usize::from(selected_venue == "MUNC"))
            .create();
        let placement = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("placeOrder".into()))
            .expect(0)
            .create();
        let error = execute_test_trade_command(
            "sc broker trade sell --isin DE0007100000 --shares 2 --order-type trailing-stop --trail-percent 5 --json",
            &config,
            &mut manager,
        ).unwrap_err();
        assert!(error.to_string().contains(if selected_venue == "XETR" {
            "require EIX or gettex"
        } else {
            "positive bid price"
        }));
        resolve.assert();
        tradability.assert();
        quote.assert();
        placement.assert();
    }
}
