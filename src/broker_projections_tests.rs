use super::*;
use crate::broker_queries::BrokerInput;
use serde_json::json;

fn broker_input(include_year_to_date: bool, quote_source: Option<&str>) -> BrokerInput {
    BrokerInput::new("acc-1", "port-1", include_year_to_date, quote_source).expect("input")
}

#[test]
fn project_broker_analytics_maps_result_sections() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "portfolioAnalysis": {
                    "type": "SUCCESS",
                    "result": {
                        "id": "analysis-1",
                        "lastUpdated": {"time":"2026-03-11T10:00:00Z"},
                        "healthChecks": {
                            "id": "checks-1",
                            "items": [
                                {
                                    "id": "hc-1",
                                    "type": "REGION",
                                    "healthScore": "0.80",
                                    "state": "HIGH",
                                    "color": {"sixDigitNotationValue":"#123456"},
                                    "numberOfItemsInPortfolio": "5",
                                    "maxItems": "10"
                                }
                            ]
                        },
                        "scenarios": {
                            "id": "scenarios-1",
                            "items": [
                                {
                                    "id": "scenario-1",
                                    "type": "WORLD_DOWN",
                                    "portfolioPerformance": "-11.5",
                                    "benchmarkPerformance": "-9.0",
                                    "securities": [
                                        {"id":"sec-1","isin":"US0378331005","name":"Apple","type":"STOCK"}
                                    ]
                                }
                            ]
                        },
                        "allocations": {
                            "id": "allocations-1",
                            "items": [
                                {
                                    "id": "allocation-1",
                                    "type": "REGION",
                                    "positions": [
                                        {
                                            "id": "position-1",
                                            "name": "North America",
                                            "weight": "0.60",
                                            "valuation": "600",
                                            "contributors": [
                                                {
                                                    "id": "contributor-1",
                                                    "weight": "0.25",
                                                    "underlyingAsset": {
                                                        "isin": "US0378331005",
                                                        "name": "Apple",
                                                        "inventory": {
                                                            "position": {
                                                                "filled": "2"
                                                            }
                                                        }
                                                    }
                                                }
                                            ],
                                            "subpositions": [
                                                {
                                                    "id": "subposition-1",
                                                    "name": "USA",
                                                    "weight": "0.50",
                                                    "valuation": "500",
                                                    "contributors": [
                                                        {
                                                            "id": "contributor-2",
                                                            "weight": "0.10",
                                                            "underlyingAsset": {
                                                                "ticker": "BTC",
                                                                "name": "Bitcoin"
                                                            }
                                                        }
                                                    ]
                                                }
                                            ]
                                        }
                                    ]
                                }
                            ]
                        },
                        "equityCompanyStyles": {
                            "id": "styles-1",
                            "marketCaps": [
                                {
                                    "id": "cap-1",
                                    "type": "LARGE",
                                    "weight": "0.70",
                                    "items": [
                                        {
                                            "id": "style-item-1",
                                            "type": "VALUE",
                                            "weight": "0.30",
                                            "contributors": [
                                                {
                                                    "id": "contributor-3",
                                                    "weight": "0.15",
                                                    "underlyingAsset": {
                                                        "isin": "US5949181045",
                                                        "name": "Microsoft"
                                                    }
                                                }
                                            ]
                                        }
                                    ],
                                    "contributors": [
                                        {
                                            "id": "contributor-4",
                                            "weight": "0.20",
                                            "underlyingAsset": {
                                                "ticker": "ETH",
                                                "name": "Ethereum"
                                            }
                                        }
                                    ]
                                }
                            ]
                        },
                        "fixedIncomeRatings": {
                            "id": "ratings-1",
                            "investmentGrade": [
                                {
                                    "id": "grade-1",
                                    "name": "AAA",
                                    "weight": "0.40",
                                    "contributors": [
                                        {
                                            "id": "contributor-5",
                                            "weight": "0.20",
                                            "underlyingAsset": {
                                                "isin": "DE0001102531",
                                                "name": "Bund",
                                                "inventory": {
                                                    "position": {
                                                        "filled": "4"
                                                    }
                                                }
                                            }
                                        }
                                    ]
                                }
                            ],
                            "speculativeGrade": [],
                            "unratedGrade": [],
                            "showSpeculativeInvestmentWarning": false
                        },
                        "payments": {
                            "id": "payments-1",
                            "totalDistributions": "12.34",
                            "totalInterest": "1.23"
                        },
                        "trialPeriod": {
                            "id": "trial-1",
                            "isEligible": true,
                            "isRunning": false,
                            "startDate": {"time":"2026-03-01T00:00:00Z"},
                            "durationInHours": 336
                        }
                    }
                }
            }
        }
    });

    let projected = project_broker_analytics_response(&input, &response).expect("project");
    assert_eq!(projected["analysis_type"], "SUCCESS");
    assert_eq!(projected["last_updated_utc"], "2026-03-11T10:00:00Z");
    assert_eq!(projected["health_checks"][0]["color_hex"], "#123456");
    assert_eq!(
        projected["scenarios"][0]["securities"][0]["security_type"],
        "STOCK"
    );
    assert_eq!(
        projected["allocations"][0]["positions"][0]["contributors"][0]["underlying_asset"]["filled_quantity"],
        "2"
    );
    assert_eq!(
        projected["allocations"][0]["positions"][0]["subpositions"][0]["contributors"][0]["underlying_asset"]
            ["kind"],
        "crypto"
    );
    assert_eq!(
        projected["equity_company_styles"]["market_caps"][0]["items"][0]["contributors"][0]["underlying_asset"]
            ["kind"],
        "security"
    );
    assert_eq!(
        projected["fixed_income_ratings"]["investment_grade"][0]["contributors"][0]["underlying_asset"]
            ["isin"],
        "DE0001102531"
    );
    assert_eq!(projected["payments"]["total_distributions"], "12.34");
    assert_eq!(
        projected["trial_period"]["start_time_utc"],
        "2026-03-01T00:00:00Z"
    );
}

#[test]
fn project_broker_analytics_maps_invalid_securities() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "portfolioAnalysis": {
                    "type": "INVALID_SECURITIES",
                    "invalidSecurities": [
                        {"id":"sec-1","isin":"US0000000001","name":"Invalid One","type":"STOCK"}
                    ],
                    "portfolioCoverage": "0.87",
                    "result": {
                        "id": "analysis-2",
                        "lastUpdated": {"time":"2026-03-11T11:00:00Z"},
                        "healthChecks": {"id":"checks-2","items":[]},
                        "scenarios": {"id":"scenarios-2","items":[]},
                        "allocations": {"id":"allocations-2","items":[]},
                        "equityCompanyStyles": {"id":"styles-2","marketCaps":[]},
                        "fixedIncomeRatings": {
                            "id":"ratings-2",
                            "investmentGrade":[],
                            "speculativeGrade":[],
                            "unratedGrade":[],
                            "showSpeculativeInvestmentWarning": true
                        },
                        "payments": null,
                        "trialPeriod": null
                    }
                }
            }
        }
    });

    let projected = project_broker_analytics_response(&input, &response).expect("project");
    assert_eq!(projected["analysis_type"], "INVALID_SECURITIES");
    assert_eq!(projected["portfolio_coverage"], "0.87");
    assert_eq!(projected["invalid_securities_count"], 1);
    assert_eq!(projected["invalid_securities"][0]["isin"], "US0000000001");
    assert_eq!(projected["payments"], json!(null));
    assert_eq!(projected["trial_period"], json!(null));
}

#[test]
fn project_broker_holdings_sorts_items() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "inventory": {
                    "items": [
                        {"isin":"B","name":"Beta","type":"ETF","inventory":{"position":{}},"portfolioIsinPerformance":{},"quoteTick":{}},
                        {"isin":"A","name":"Alpha","type":"STOCK","inventory":{"position":{}},"portfolioIsinPerformance":{},"quoteTick":{}}
                    ]
                }
            }
        }
    });

    let projected = project_broker_holdings_response(&input, &response).expect("project");
    assert_eq!(projected["count"], 2);
    assert_eq!(projected["items"][0]["isin"], "A");
}

#[test]
fn project_broker_search_includes_query() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "simpleSecuritySearch": {
                    "items": [{"isin":"A","name":"Alpha","type":"STOCK","quoteTick":{}}]
                }
            }
        }
    });

    let projected = project_broker_search_response(&input, "apple", &response).expect("project");
    assert_eq!(projected["query"], "apple");
    assert_eq!(projected["count"], 1);
}

#[test]
fn project_broker_derivatives_search_maps_price_and_expiry_shapes() {
    let input = broker_input(false, None);
    let query = crate::broker_queries::NormalizedBrokerDerivativesSearchQueryInput {
        derivative_type: crate::cli::BrokerDerivativeType::Knockout,
        underlying_isin: "US0378331005".to_string(),
        limit: 25,
        offset: 50,
        issuers: Some(vec!["HSBC".to_string()]),
        strategy: "LONG".to_string(),
        product_subcategories: Some(vec!["TURBO".to_string()]),
        leverage_range: None,
        knockout_barrier_range: None,
        strike_range: None,
        omega_range: None,
        delta_range: None,
        factor_range: None,
        expiry_date_range: None,
        sort_by: None,
    };
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "derivativesSearch": {
                    "pagination": {
                        "offset": 50,
                        "limit": 25,
                        "totalAvailable": 2
                    },
                    "results": [
                        {
                            "__typename": "KnockoutSearchResult",
                            "isin": "DE000HSBC123",
                            "underlyingIsin": "US0378331005",
                            "issuer": "HSBC",
                            "strategy": "LONG",
                            "productSubcategory": "TURBO",
                            "leverage": "4.2",
                            "distanceToKnockout": "3.5",
                            "distanceToStrike": "1.2",
                            "strike": {
                                "__typename": "Money",
                                "currencyIsoCode": "EUR",
                                "value": "190.00"
                            },
                            "knockoutBarrier": {
                                "__typename": "Point",
                                "value": "18000"
                            },
                            "premiumAbsolute": {
                                "currencyIsoCode": "EUR",
                                "value": "7.40"
                            },
                            "premiumPercentage": "0.0415",
                            "expiryDate": {
                                "date": {
                                    "date": "2026-12-31",
                                    "epochDay": 20818
                                },
                                "isOpenEnd": false
                            }
                        },
                        {
                            "__typename": "WarrantSearchResult",
                            "isin": "DE000WARRANT1",
                            "underlyingIsin": "US0378331005",
                            "issuer": "BNP",
                            "strategy": "CALL",
                            "omega": "8.5",
                            "delta": "0.40",
                            "impliedVolatility": "0.25",
                            "distanceToStrike": "4.2",
                            "strike": {
                                "__typename": "Money",
                                "currencyIsoCode": "USD",
                                "value": "210.00"
                            },
                            "expiryDate": {
                                "epochDay": 20714
                            }
                        },
                        {
                            "__typename": "FactorCertificateSearchResult",
                            "isin": "DE000FACTOR1",
                            "underlyingIsin": "US0378331005",
                            "issuer": "SOC_GEN",
                            "strategy": "SHORT",
                            "factor": "5",
                            "expiryDate": {
                                "date": {
                                    "date": "2027-01-15",
                                    "epochDay": 20833
                                },
                                "isOpenEnd": false
                            }
                        }
                    ]
                }
            }
        }
    });

    let projected =
        project_broker_derivatives_search_response(&input, &query, &response).expect("project");
    assert_eq!(projected["derivative_type"], "knockout");
    assert_eq!(projected["underlying_isin"], "US0378331005");
    assert_eq!(projected["total_available"], 2);
    assert_eq!(projected["items"][0]["strike"]["kind"], "money");
    assert_eq!(projected["items"][0]["knockout_barrier"]["kind"], "point");
    assert_eq!(projected["items"][0]["expiry_is_open_end"], false);
    assert_eq!(projected["items"][0]["expiry_date"]["date"], "2026-12-31");
    assert_eq!(projected["items"][1]["omega"], "8.5");
    assert_eq!(projected["items"][1]["implied_volatility"], "0.25");
    assert_eq!(projected["items"][1]["expiry_date"]["epoch_day"], 20714);
    assert_eq!(projected["items"][1]["expiry_date"]["date"], json!(null));
    assert_eq!(projected["items"][1]["expiry_is_open_end"], json!(null));
    assert_eq!(projected["items"][2]["factor"], "5");
    assert_eq!(projected["items"][2]["expiry_date"]["epoch_day"], 20833);
    assert_eq!(projected["items"][2]["expiry_is_open_end"], false);
}

#[test]
fn project_broker_quote_response_maps_quote_fields() {
    let input = broker_input(true, Some("CONSOLIDATED"));
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "security": {
                    "id": "security-1",
                    "isin": "US0378331005",
                    "name": "Apple",
                    "type": "STOCK",
                    "quoteTick": {
                        "id": "tick-1",
                        "midPrice": 201.1,
                        "bidPrice": 201.0,
                        "askPrice": 201.2,
                        "currency": "USD",
                        "timestampUtc": {
                            "time": "2026-03-11T08:00:00Z"
                        },
                        "isOutdated": false,
                        "performanceDate": {
                            "date": "2026-03-11"
                        },
                        "performancesByTimeframe": [
                            {
                                "timeframe": "ONE_WEEK",
                                "performance": 0.02,
                                "simpleAbsoluteReturn": 4.0
                            },
                            {
                                "timeframe": "ONE_DAY",
                                "performance": 0.01,
                                "simpleAbsoluteReturn": 2.0
                            }
                        ]
                    }
                }
            }
        }
    });

    let projected =
        project_broker_quote_response(&input, "US0378331005", &response).expect("project");
    assert_eq!(projected["security_id"], "security-1");
    assert_eq!(projected["isin"], "US0378331005");
    assert_eq!(projected["name"], "Apple");
    assert_eq!(projected["quote_tick_id"], "tick-1");
    assert_eq!(projected["quote_mid_price"], 201.1);
    assert_eq!(projected["quote_bid_price"], 201.0);
    assert_eq!(projected["quote_ask_price"], 201.2);
    assert_eq!(projected["quote_currency"], "USD");
    assert_eq!(projected["quote_timestamp_utc"], "2026-03-11T08:00:00Z");
    assert_eq!(projected["quote_performance_date"], "2026-03-11");
    assert_eq!(projected["quote_performances"][0]["timeframe"], "ONE_DAY");
    assert_eq!(projected["quote_performances"][1]["timeframe"], "ONE_WEEK");
}

#[test]
fn project_broker_quote_response_requires_matching_returned_isin() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "security": {
                    "isin": "DE0007100000",
                    "quoteTick": {}
                }
            }
        }
    });

    let err = project_broker_quote_response(&input, "US0378331005", &response).unwrap_err();
    assert!(
        err.to_string()
            .contains("does not match returned security isin")
    );
}

#[test]
fn project_broker_quote_response_requires_security_node() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {}
        }
    });

    let err = project_broker_quote_response(&input, "US0378331005", &response).unwrap_err();
    assert!(err.to_string().contains("account.brokerPortfolio.security"));
}

#[test]
fn project_broker_chart_response_maps_series_fields() {
    let response = json!({
        "timeSeriesBySecurity": [
            {
                "isin": "US0378331005",
                "timeFrame": "ONE_MONTH",
                "currency": "EUR",
                "source": "CONSOLIDATED",
                "closingReferencePoint": {
                    "midPrice": 184.12,
                    "timestampUtc": {
                        "time": "2026-05-22T21:59:59Z"
                    }
                },
                "dataPoints": [
                    {
                        "midPrice": 185.01,
                        "timestampUtc": {
                            "time": "2026-05-23T09:00:00Z"
                        }
                    },
                    {
                        "midPrice": 183.50,
                        "timestampUtc": {
                            "time": "2026-05-24T09:00:00Z"
                        }
                    }
                ]
            }
        ]
    });

    let projected = project_broker_chart_response(
        "US0378331005",
        crate::cli::BrokerChartTimeframe::OneMonth,
        &response,
    )
    .expect("project");

    assert_eq!(projected["isin"], "US0378331005");
    assert_eq!(projected["timeframe"], "1m");
    assert_eq!(projected["currency"], "EUR");
    assert_eq!(projected["source"], "CONSOLIDATED");
    assert_eq!(projected["closing_reference_point"]["mid_price"], 184.12);
    assert_eq!(
        projected["closing_reference_point"]["timestamp_utc"],
        "2026-05-22T21:59:59Z"
    );
    assert_eq!(projected["data_points"][0]["mid_price"], 185.01);
    assert_eq!(
        projected["data_points"][0]["timestamp_utc"],
        "2026-05-23T09:00:00Z"
    );
    assert_eq!(projected["data_points"][1]["mid_price"], 183.50);
    assert_eq!(
        projected["data_points"][1]["timestamp_utc"],
        "2026-05-24T09:00:00Z"
    );
    assert_eq!(projected["point_count"], 2);
}

#[test]
fn project_broker_chart_response_returns_empty_data_points_successfully() {
    let response = json!({
        "timeSeriesBySecurity": [
            {
                "isin": "US0378331005",
                "timeFrame": "YEAR_TO_DATE",
                "currency": "EUR",
                "source": "CONSOLIDATED",
                "closingReferencePoint": {
                    "midPrice": 184.12,
                    "timestampUtc": {
                        "time": "2026-05-22T21:59:59Z"
                    }
                },
                "dataPoints": []
            }
        ]
    });

    let projected = project_broker_chart_response(
        "US0378331005",
        crate::cli::BrokerChartTimeframe::YearToDate,
        &response,
    )
    .expect("project");

    assert_eq!(projected["data_points"], json!([]));
    assert_eq!(projected["point_count"], 0);
}

#[test]
fn project_broker_chart_response_requires_requested_timeframe() {
    let response = json!({
        "timeSeriesBySecurity": [
            {
                "isin": "US0378331005",
                "timeFrame": "ONE_MONTH",
                "currency": "EUR",
                "source": "CONSOLIDATED",
                "closingReferencePoint": {},
                "dataPoints": []
            }
        ]
    });

    let err = project_broker_chart_response(
        "US0378331005",
        crate::cli::BrokerChartTimeframe::YearToDate,
        &response,
    )
    .unwrap_err();

    assert!(err.to_string().contains("requested timeframe 'ytd'"));
}

#[test]
fn project_broker_chart_response_requires_matching_returned_isin() {
    let response = json!({
        "timeSeriesBySecurity": [
            {
                "isin": "DE000BASF111",
                "timeFrame": "ONE_MONTH",
                "currency": "EUR",
                "source": "CONSOLIDATED",
                "closingReferencePoint": {},
                "dataPoints": []
            }
        ]
    });

    let err = project_broker_chart_response(
        "US0378331005",
        crate::cli::BrokerChartTimeframe::OneMonth,
        &response,
    )
    .unwrap_err();

    assert!(err.to_string().contains("requested isin 'US0378331005'"));
}

#[test]
fn project_broker_watchlist_add_maps_minimal_ack() {
    let response = json!({
        "addToWatchlist": {
            "security": {
                "isin": "US0378331005",
                "isOnWatchlist": true
            }
        }
    });

    let projected =
        project_broker_watchlist_add_response("US0378331005", &response).expect("project");
    assert_eq!(projected["action"], "add");
    assert_eq!(projected["isin"], "US0378331005");
    assert_eq!(projected["is_on_watchlist"], true);
}

#[test]
fn project_broker_watchlist_remove_maps_minimal_ack() {
    let response = json!({
        "removeFromWatchlist": {
            "security": {
                "isin": "US0378331005",
                "isOnWatchlist": false
            }
        }
    });

    let projected =
        project_broker_watchlist_remove_response("US0378331005", &response).expect("project");
    assert_eq!(projected["action"], "remove");
    assert_eq!(projected["isin"], "US0378331005");
    assert_eq!(projected["is_on_watchlist"], false);
}

#[test]
fn project_broker_remove_savings_plan_response_maps_minimal_ack() {
    let response = json!({
        "removeSavingsPlan": {
            "id": "mutation-1"
        }
    });

    let projected =
        project_broker_remove_savings_plan_response("US0378331005", &response).expect("project");
    assert_eq!(projected["action"], "remove");
    assert_eq!(projected["isin"], "US0378331005");
}

#[test]
fn project_broker_watchlist_mutations_require_security() {
    let add_err = project_broker_watchlist_add_response(
        "US0378331005",
        &json!({
            "addToWatchlist": {}
        }),
    )
    .unwrap_err();
    assert!(add_err.to_string().contains("Broker response invalid"));

    let remove_err = project_broker_watchlist_remove_response(
        "US0378331005",
        &json!({
            "removeFromWatchlist": {}
        }),
    )
    .unwrap_err();
    assert!(remove_err.to_string().contains("Broker response invalid"));
}

#[test]
fn project_broker_remove_savings_plan_response_requires_mutation_field() {
    let err = project_broker_remove_savings_plan_response(
        "US0378331005",
        &json!({
            "otherField": {}
        }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("removeSavingsPlan"));
}

#[test]
fn project_broker_remove_savings_plan_response_rejects_null_mutation() {
    let err = project_broker_remove_savings_plan_response(
        "US0378331005",
        &json!({
            "removeSavingsPlan": null
        }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("removeSavingsPlan.id"));
}

#[test]
fn project_broker_remove_savings_plan_response_requires_non_empty_id() {
    let err = project_broker_remove_savings_plan_response(
        "US0378331005",
        &json!({
            "removeSavingsPlan": {
                "id": "   "
            }
        }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("removeSavingsPlan.id"));
}

#[test]
fn project_broker_transactions_maps_variants_and_preserves_order() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "moreTransactions": {
                    "cursor": "next-cursor",
                    "total": 3,
                    "transactions": [
                        {
                            "__typename": "BrokerCashTransactionSummary",
                            "id": "tx-1",
                            "currency": "EUR",
                            "type": "CASH_TRANSACTION",
                            "status": "SETTLED",
                            "isCancellation": false,
                            "lastEventDateTime": {"time":"2026-03-10T10:00:00Z"},
                            "description": "Deposit",
                            "custodian": "BAADER_BANK",
                            "documents": [],
                            "relatedIsin": null,
                            "cashTransactionType": "DEPOSIT",
                            "amount": "100"
                        },
                        {
                            "__typename": "BrokerSecurityTransactionSummary",
                            "id": "tx-2",
                            "currency": "EUR",
                            "type": "SECURITY_TRANSACTION",
                            "status": "FILLED",
                            "isCancellation": false,
                            "lastEventDateTime": {"time":"2026-03-09T10:00:00Z"},
                            "description": "Buy",
                            "custodian": "BAADER_BANK",
                            "documents": [],
                            "isin": "US0378331005",
                            "securityTransactionType": "SINGLE",
                            "quantity": "1.23",
                            "amount": "200",
                            "side": "BUY",
                            "limitPrice": null,
                            "stopPrice": null
                        },
                        {
                            "__typename": "FutureTransactionSummary",
                            "id": "tx-3",
                            "currency": "EUR",
                            "type": "CASH_TRANSACTION",
                            "status": "CREATED",
                            "isCancellation": false,
                            "lastEventDateTime": null,
                            "description": "Unknown variant",
                            "custodian": null,
                            "documents": []
                        }
                    ]
                }
            }
        }
    });

    let projected = project_broker_transactions_response(&input, &response).expect("project");
    assert_eq!(projected["cursor"], "next-cursor");
    assert_eq!(projected["total"], 3);
    assert_eq!(projected["count"], 3);
    assert_eq!(projected["items"][0]["id"], "tx-1");
    assert_eq!(projected["items"][0]["cash_transaction_type"], "DEPOSIT");
    assert_eq!(projected["items"][1]["id"], "tx-2");
    assert_eq!(projected["items"][1]["security_transaction_type"], "SINGLE");
    assert_eq!(projected["items"][2]["id"], "tx-3");
    assert_eq!(projected["items"][2]["unknown_summary_type"], true);
}

#[test]
fn project_broker_transaction_details_maps_security_trade_tax_fields() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "transactionDetails": {
                    "__typename": "BrokerSecurityTransaction",
                    "id": "tx-42",
                    "currency": "EUR",
                    "type": "SECURITY_TRANSACTION",
                    "documents": [
                        {
                            "id": "doc-1",
                            "url": "https://example.test/doc-1",
                            "label": "Contract note"
                        }
                    ],
                    "lastEventDateTime": {"time":"2026-04-15T10:22:31Z"},
                    "isPending": false,
                    "isCancellation": false,
                    "security": {
                        "isin": "IE00B4ND3602",
                        "name": "World ETF",
                        "type": "ETF"
                    },
                    "transactionReference": "WUM 872598752",
                    "side": "SELL",
                    "status": "FILLED",
                    "numberOfShares": {
                        "filled": "10",
                        "total": "10"
                    },
                    "averagePrice": "10.25",
                    "totalAmount": "100.00",
                    "finalisationReason": null,
                    "limitPrice": null,
                    "stopPrice": null,
                    "validUntil": null,
                    "isCancellationRequested": false,
                    "tradeTransactionAmounts": {
                        "marketValuation": "102.50",
                        "taxAmount": "2.50",
                        "transactionFee": "0.99",
                        "venueFee": "1.01",
                        "cryptoSpreadFee": null
                    },
                    "tradingVenue": "MUNC",
                    "fee": "1.11",
                    "transactionalFee": "0.22",
                    "taxes": "2.50",
                    "aggregatedTransactionTaxes": {
                        "totalTax": "2.50",
                        "capitalGainsTax": "2.10",
                        "churchTax": "0.10",
                        "solidarityTax": "0.30",
                        "sourceTax": null,
                        "financialTransactionTax": null
                    },
                    "securityTransactionHistory": [
                        {
                            "state": "FILLED",
                            "time": {
                                "time": "2026-04-15T10:22:31",
                                "epochSecond": 1776248551,
                                "epochMillisecond": 1776248551000_i64
                            },
                            "numberOfShares": {
                                "filled": "10",
                                "total": "10"
                            },
                            "executionPrice": "10.25"
                        }
                    ],
                    "orderKind": "SINGLE",
                    "linkedTransactions": [
                        {"id": "tx-43"},
                        {"id": "tx-44"}
                    ],
                    "trailingStopInfo": {
                        "trailType": "AMOUNT",
                        "trailOffset": "1.50",
                        "latestStopPriceTimestamp": {
                            "time": "2026-04-15T10:23:00Z",
                            "epochSecond": 1776248580,
                            "epochMillisecond": 1776248580000_i64
                        }
                    }
                }
            }
        }
    });

    let projected =
        project_broker_transaction_details_response(&input, &response).expect("project");
    assert_eq!(projected["id"], "tx-42");
    assert_eq!(projected["detail_type"], "security_trade");
    assert_eq!(projected["detail_typename"], "BrokerSecurityTransaction");
    assert_eq!(projected["transaction_reference"], "WUM 872598752");
    assert_eq!(projected["security"]["isin"], "IE00B4ND3602");
    assert_eq!(
        projected["linked_transaction_ids"],
        json!(["tx-43", "tx-44"])
    );
    assert_eq!(projected["history"][0]["timestamp"], "2026-04-15T10:22:31");
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_second")
            .is_none()
    );
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_millisecond")
            .is_none()
    );
    assert_eq!(
        projected["security_trade"]["trade_transaction_amounts"]["tax_amount"],
        "2.50"
    );
    assert_eq!(
        projected["security_trade"]["aggregated_transaction_taxes"]["capital_gains_tax"],
        "2.10"
    );
    assert_eq!(projected["security_trade"]["taxes"], "2.50");
    assert_eq!(projected["cash"], json!(null));
    assert_eq!(projected["eltif"], json!(null));
}

#[test]
fn project_broker_transaction_details_maps_cash_transaction_fields() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "transactionDetails": {
                    "__typename": "BrokerCashTransaction",
                    "id": "cash-1",
                    "currency": "EUR",
                    "type": "CASH_TRANSACTION",
                    "documents": [],
                    "lastEventDateTime": {"time":"2026-04-17T09:30:00Z"},
                    "isPending": false,
                    "isCancellation": false,
                    "security": {
                        "isin": "DE0001234567",
                        "name": "Related Security",
                        "type": "BOND"
                    },
                    "transactionReference": "CASH-REF-1",
                    "cashTransactionType": "TAX",
                    "amount": "25.00",
                    "description": "Tax withholding",
                    "cashTransactionHistory": [
                        {
                            "state": "BOOKED",
                            "time": {
                                "time": "2026-04-17T09:30:00",
                                "epochSecond": 1776418200,
                                "epochMillisecond": 1776418200000_i64
                            }
                        }
                    ],
                    "sddiDetails": {
                        "fee": "0.50",
                        "grossAmount": "25.50"
                    },
                    "taxDetails": {
                        "grossAmount": "25.50",
                        "taxAmount": "0.50"
                    },
                    "linkedTransactions": [
                        {"id": "cash-2"}
                    ]
                }
            }
        }
    });

    let projected =
        project_broker_transaction_details_response(&input, &response).expect("project");
    assert_eq!(projected["detail_type"], "cash");
    assert_eq!(projected["detail_typename"], "BrokerCashTransaction");
    assert_eq!(projected["linked_transaction_ids"], json!(["cash-2"]));
    assert_eq!(projected["history"][0]["state"], "BOOKED");
    assert_eq!(projected["history"][0]["timestamp"], "2026-04-17T09:30:00");
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_second")
            .is_none()
    );
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_millisecond")
            .is_none()
    );
    assert_eq!(projected["cash"]["cash_transaction_type"], "TAX");
    assert_eq!(projected["cash"]["amount"], "25.00");
    assert_eq!(projected["cash"]["tax_details"]["gross_amount"], "25.50");
    assert_eq!(projected["cash"]["tax_details"]["tax_amount"], "0.50");
    assert_eq!(projected["cash"]["sddi_details"]["fee"], "0.50");
    assert_eq!(projected["cash"]["sddi_details"]["gross_amount"], "25.50");
    assert_eq!(projected["security_trade"], json!(null));
    assert_eq!(projected["non_trade_security"], json!(null));
}

#[test]
fn project_broker_transaction_details_maps_non_trade_security_fields() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "transactionDetails": {
                    "__typename": "BrokerNonTradeSecurityTransaction",
                    "id": "non-trade-1",
                    "currency": "EUR",
                    "type": "SECURITY_TRANSACTION",
                    "documents": [],
                    "lastEventDateTime": {"time":"2026-04-18T11:45:00Z"},
                    "isPending": false,
                    "isCancellation": false,
                    "security": {
                        "isin": "US0000000001",
                        "name": "Inbound Transfer Security",
                        "type": "STOCK"
                    },
                    "transactionReference": "NTS-REF-1",
                    "isin": "US0000000001",
                    "nonTradeSecurityTransactionType": "TRANSFER_IN",
                    "quantity": "3.5",
                    "nonTradeAveragePrice": "12.34",
                    "nonTradeSecurityAmount": "43.19",
                    "description": "Inbound transfer",
                    "nonTradeSecurityTransactionHistory": [
                        {
                            "state": "BOOKED",
                            "time": {
                                "time": "2026-04-18T11:45:00",
                                "epochSecond": 1776512700,
                                "epochMillisecond": 1776512700000_i64
                            }
                        }
                    ],
                    "linkedTransactions": [
                        {"id": "cash-linked-1"}
                    ]
                }
            }
        }
    });

    let projected =
        project_broker_transaction_details_response(&input, &response).expect("project");
    assert_eq!(projected["detail_type"], "non_trade_security");
    assert_eq!(
        projected["detail_typename"],
        "BrokerNonTradeSecurityTransaction"
    );
    assert_eq!(
        projected["linked_transaction_ids"],
        json!(["cash-linked-1"])
    );
    assert_eq!(projected["history"][0]["state"], "BOOKED");
    assert_eq!(projected["history"][0]["timestamp"], "2026-04-18T11:45:00");
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_second")
            .is_none()
    );
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_millisecond")
            .is_none()
    );
    assert_eq!(projected["non_trade_security"]["isin"], "US0000000001");
    assert_eq!(
        projected["non_trade_security"]["non_trade_security_transaction_type"],
        "TRANSFER_IN"
    );
    assert_eq!(projected["non_trade_security"]["quantity"], "3.5");
    assert_eq!(projected["non_trade_security"]["average_price"], "12.34");
    assert_eq!(projected["non_trade_security"]["total_amount"], "43.19");
    assert_eq!(projected["cash"], json!(null));
    assert_eq!(projected["security_trade"], json!(null));
}

#[test]
fn project_broker_transaction_details_gracefully_maps_eltif() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "transactionDetails": {
                    "__typename": "BrokerEltifTransaction",
                    "id": "eltif-1",
                    "currency": "EUR",
                    "type": "SECURITY_TRANSACTION",
                    "documents": [],
                    "lastEventDateTime": {"time":"2026-04-16T12:00:00Z"},
                    "isPending": true,
                    "isCancellation": false,
                    "security": {
                        "isin": "LU0000000001",
                        "name": "Example Eltif",
                        "type": "ELTIF"
                    },
                    "transactionReference": "ELTIF-REF-1",
                    "status": "REQUESTED",
                    "side": "BUY",
                    "orderKind": "SINGLE",
                    "amount": "500.00",
                    "finalisationReason": null,
                    "eltifQuantity": "5",
                    "executionPrice": null,
                    "executionDate": null,
                    "earliestSellDate": {"time":"2028-04-16T00:00:00Z"},
                    "marketValuation": null,
                    "cancelableDetails": {
                        "daysLeft": 2,
                        "isCancelable": true
                    },
                    "isMultipleOrdersCancellation": false,
                    "isInitialInvestment": true,
                    "tradingVenue": "XVES",
                    "eltifTransactionHistory": [
                        {
                            "state": "REQUESTED",
                            "amount": "500.00",
                            "eltifQuantity": "5",
                            "executionPrice": null,
                            "time": {
                                "time": "2026-04-16T12:00:00",
                                "epochSecond": 1776340800,
                                "epochMillisecond": 1776340800000_i64
                            }
                        }
                    ],
                    "linkedTransactions": []
                }
            }
        }
    });

    let projected =
        project_broker_transaction_details_response(&input, &response).expect("project");
    assert_eq!(projected["detail_type"], "eltif");
    assert_eq!(projected["detail_typename"], "BrokerEltifTransaction");
    assert_eq!(projected["security_trade"], json!(null));
    assert_eq!(projected["cash"], json!(null));
    assert_eq!(projected["linked_transaction_ids"], json!([]));
    assert_eq!(projected["history"][0]["amount"], "500.00");
    assert_eq!(projected["history"][0]["timestamp"], "2026-04-16T12:00:00");
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_second")
            .is_none()
    );
    assert!(
        projected["history"][0]
            .get("timestamp_epoch_millisecond")
            .is_none()
    );
    assert_eq!(projected["eltif"]["amount"], "500.00");
    assert_eq!(projected["eltif"]["is_initial_investment"], true);
    assert_eq!(projected["eltif"]["cancelable_details"]["days_left"], 2);
}

#[test]
fn project_broker_security_news_sorts_sources() {
    let response = json!({
        "securityNews": {
            "shortNewsSummary": "short",
            "longNewsSummary": "long",
            "lastUpdated": "2026-03-04T10:00:00Z",
            "sources": [
                {"id": "b", "headline":"B", "sourceName":"S", "publicationTime":"t2"},
                {"id": "a", "headline":"A", "sourceName":"S", "publicationTime":"t1"}
            ]
        }
    });

    let projected =
        project_broker_security_news_response("US1", "en_DE", &response).expect("project");
    assert_eq!(projected["sources"][0]["id"], "a");
}

#[test]
fn project_broker_price_alerts_flattens() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "priceAlerts": {
                    "itemsPerInstrument": [
                        {
                            "canAddNew": true,
                            "items": [
                                {
                                    "id": "2",
                                    "direction": "UP",
                                    "isActive": true,
                                    "price": "11.0",
                                    "triggeredTimestamp": {"time":"2026-03-04T10:00:00Z"},
                                    "security": {"isin": "B", "name": "Beta", "type": "ETF"}
                                },
                                {
                                    "id": "1",
                                    "direction": "DOWN",
                                    "isActive": false,
                                    "price": "10.0",
                                    "triggeredTimestamp": null,
                                    "security": {"isin": "A", "name": "Alpha", "type": "STOCK"}
                                }
                            ]
                        }
                    ]
                }
            }
        }
    });

    let projected = project_broker_price_alerts_response(&input, true, &response).expect("project");
    assert_eq!(projected["count"], 2);
    assert_eq!(projected["items"][0]["isin"], "A");
}

#[test]
fn project_broker_crypto_price_alerts_flattens() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "crypto": {
                    "priceAlerts": {
                        "itemsPerInstrument": [
                            {
                                "canAddNew": true,
                                "items": [
                                    {
                                        "id": "2",
                                        "direction": "UP",
                                        "isActive": true,
                                        "price": "11.0",
                                        "triggeredTimestamp": {"time":"2026-03-04T10:00:00Z"},
                                        "coin": {"ticker": "ETH", "name": "Ether"}
                                    },
                                    {
                                        "id": "1",
                                        "direction": "DOWN",
                                        "isActive": false,
                                        "price": "10.0",
                                        "triggeredTimestamp": null,
                                        "coin": {"ticker": "BTC", "name": "Bitcoin"}
                                    }
                                ]
                            }
                        ]
                    }
                }
            }
        }
    });

    let projected =
        project_broker_crypto_price_alerts_response(&input, &response).expect("project");
    assert_eq!(projected["count"], 2);
    assert_eq!(projected["items"][0]["ticker"], "BTC");
}

#[test]
fn project_broker_crypto_price_alerts_returns_empty_when_crypto_tree_is_absent() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "priceAlerts": {
                    "itemsPerInstrument": []
                }
            }
        }
    });

    let projected =
        project_broker_crypto_price_alerts_response(&input, &response).expect("project");
    assert_eq!(projected["count"], 0);
    assert_eq!(projected["items"], json!([]));
}

#[test]
fn project_broker_cash_breakdown_includes_only_approved_fields() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "depositLimits": {"min": "1", "max": "2"},
                "withdrawalLimits": {"min": "3", "max": "4", "maxExcludingCredit": "5"},
                "payments": {
                    "buyingPower": {
                        "cashBalance": "10",
                        "liveLimit": "20",
                        "loaned": "30",
                        "pendingBuyOrdersAmount": "40",
                        "pendingWithdrawalsAmount": "50",
                        "pendingSavingsPlanAmount": "60",
                        "pendingDividendsReinvestmentAmount": "70",
                        "pendingPocketMoneyAmount": "80",
                        "estimatedTaxes": "90",
                        "directDebit": "100",
                        "cashAvailableToInvest": "110",
                        "cashAvailableToInvestWithoutCredit": "120"
                    },
                    "derivativesBuyingPower": {
                        "cashAvailableToInvest": "130",
                        "derivativesDirectDebit": "140",
                        "pendingELTIFAmount": "150",
                        "cashAvailableForDerivatives": "160"
                    },
                    "withdrawalPower": {
                        "cashAvailableToInvest": "170",
                        "sellTradesAmount": "180",
                        "withdrawalDirectDebit": "190",
                        "cashAvailableForWithdrawal": "200"
                    }
                }
            }
        }
    });

    let projected = project_broker_cash_breakdown_response(&input, &response).expect("project");

    assert_eq!(
        projected,
        json!({
            "account_id": "acc-1",
            "portfolio_id": "port-1",
            "cash_balance": "10",
            "buying_power": "110",
            "buying_power_without_credit": "120",
            "available_credit_line": "20",
            "loaned": "30",
            "pending_buy_orders_amount": "40",
            "possible_taxes": "90",
            "derivatives_buying_power": "130",
            "available_for_derivatives": "160"
        })
    );
    assert!(projected.get("currency").is_none());
    assert!(projected.get("deposit_limits").is_none());
    assert!(projected.get("withdrawal_limits").is_none());
    assert!(projected.get("withdrawal_power").is_none());
    assert!(projected.get("pending_withdrawals_amount").is_none());
    assert!(projected.get("pending_eltif_amount").is_none());
}

#[test]
fn project_broker_cash_breakdown_returns_nulls_when_payments_are_missing() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {}
        }
    });

    let projected = project_broker_cash_breakdown_response(&input, &response).expect("project");

    assert_eq!(
        projected,
        json!({
            "account_id": "acc-1",
            "portfolio_id": "port-1",
            "cash_balance": null,
            "buying_power": null,
            "buying_power_without_credit": null,
            "available_credit_line": null,
            "loaned": null,
            "pending_buy_orders_amount": null,
            "possible_taxes": null,
            "derivatives_buying_power": null,
            "available_for_derivatives": null
        })
    );
}

#[test]
fn project_broker_savings_plans_flattens_and_filters() {
    let input = broker_input(false, None);
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "totalSavingsPlanAmount": "260",
                "inventory": {
                    "items": [
                        {
                            "isin": "B",
                            "name": "Beta",
                            "type": "ETF",
                            "inventory": {
                                "savingsPlan": null
                            }
                        },
                        {
                            "isin": "A",
                            "name": "Alpha",
                            "type": "STOCK",
                            "inventory": {
                                "savingsPlan": {
                                    "amount": "100",
                                    "frequency": "MONTHLY",
                                    "dayOfTheMonth": 5,
                                    "dynamizationRate": "0",
                                    "paymentMethod": "REFERENCE_ACCOUNT",
                                    "nextExecutionDate": {
                                        "date": "2026-04-05",
                                        "epochDay": 20549
                                    }
                                }
                            }
                        }
                    ]
                },
                "crypto": {
                    "coins": [
                        {"ticker": "ETH", "name": "Ethereum", "savingsPlanAmount": "0"},
                        {"ticker": "BTC", "name": "Bitcoin", "savingsPlanAmount": "160"}
                    ]
                }
            }
        }
    });

    let projected = project_broker_savings_plans_response(&input, &response).expect("project");
    assert_eq!(projected["total_savings_plan_amount"], "260");
    assert_eq!(projected["count"], 2);
    assert_eq!(projected["non_crypto_count"], 1);
    assert_eq!(projected["crypto_count"], 1);
    assert_eq!(projected["items"][0]["kind"], "security");
    assert_eq!(projected["items"][0]["isin"], "A");
    assert_eq!(projected["items"][1]["kind"], "crypto");
    assert_eq!(projected["items"][1]["ticker"], "BTC");
}

#[test]
fn project_broker_savings_plan_config_extracts_config() {
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "security": {
                    "savingsPlanConfiguration": {
                        "frequencies": ["MONTHLY"]
                    }
                }
            }
        }
    });
    let projected = project_broker_savings_plan_config_response(&response).expect("project");
    assert_eq!(projected["frequencies"][0], "MONTHLY");
}

#[test]
fn project_broker_create_or_update_savings_plan_response_maps_mutation_id() {
    let response = json!({
        "createOrUpdateSavingsPlan": {
            "id": "mutation-1"
        }
    });
    let projected =
        project_broker_create_or_update_savings_plan_response(&response).expect("project");
    assert_eq!(projected["mutation_id"], "mutation-1");
}

#[test]
fn project_broker_savings_plan_by_isin_response_maps_security_and_plan() {
    let response = json!({
        "account": {
            "brokerPortfolio": {
                "security": {
                    "isin": "US0378331005",
                    "name": "Apple",
                    "type": "STOCK",
                    "inventory": {
                        "savingsPlan": {
                            "isin": "US0378331005",
                            "amount": "100",
                            "frequency": "MONTHLY",
                            "dayOfTheMonth": 5,
                            "dynamizationRate": "0",
                            "paymentMethod": "REFERENCE_ACCOUNT",
                            "nextExecutionDate": {
                                "date": "2026-04-05",
                                "epochDay": 20549
                            }
                        }
                    }
                }
            }
        }
    });
    let projected = project_broker_savings_plan_by_isin_response(&response).expect("project");
    assert_eq!(projected["security"]["isin"], "US0378331005");
    assert_eq!(projected["savings_plan"]["frequency"], "MONTHLY");
}

#[test]
fn project_broker_add_price_alert_response_maps_items() {
    let input = broker_input(false, None);
    let response = json!({
        "addPriceAlert": {
            "security": {
                "priceAlerts": {
                    "canAddNew": false,
                    "items": [
                        {"id":"b","direction":"UP","isActive":true,"price":"101","triggeredTimestamp":{"time":"2026-03-04T10:00:00Z"},"security":{"isin":"US0378331005","name":"Apple","type":"STOCK"}},
                        {"id":"a","direction":"DOWN","isActive":false,"price":"99","triggeredTimestamp":null,"security":{"isin":"US0378331005","name":"Apple","type":"STOCK"}}
                    ]
                }
            }
        }
    });
    let projected =
        project_broker_add_price_alert_response(&input, "US0378331005", "101", &response)
            .expect("project");
    assert_eq!(projected["kind"], "security");
    assert_eq!(projected["count"], 2);
    assert_eq!(projected["items"][0]["alert_id"], "a");
}

#[test]
fn project_broker_add_crypto_price_alert_response_maps_items() {
    let input = broker_input(false, None);
    let response = json!({
        "addCryptoPriceAlert": {
            "crypto": {
                "coin": {
                    "ticker": "BTC",
                    "name": "Bitcoin",
                    "priceAlerts": {
                        "canAddNew": true,
                        "items": [
                            {"id":"b","direction":"UP","isActive":true,"price":"101","triggeredTimestamp":{"time":"2026-03-04T10:00:00Z"},"coin":{"ticker":"BTC","name":"Bitcoin"}},
                            {"id":"a","direction":"DOWN","isActive":false,"price":"99","triggeredTimestamp":null,"coin":{"ticker":"BTC","name":"Bitcoin"}}
                        ]
                    }
                }
            }
        }
    });
    let projected = project_broker_add_crypto_price_alert_response(&input, "BTC", "101", &response)
        .expect("project");
    assert_eq!(projected["kind"], "crypto");
    assert_eq!(projected["count"], 2);
    assert_eq!(projected["items"][0]["alert_id"], "a");
}

#[test]
fn project_broker_remove_price_alert_response_maps_minimal_ack() {
    let response = json!({
        "removePriceAlert": {
            "id": "alert-1"
        }
    });

    let projected =
        project_broker_remove_price_alert_response("alert-1", "US0378331005", &response)
            .expect("project");
    assert_eq!(projected["action"], "remove");
    assert_eq!(projected["kind"], "security");
    assert_eq!(projected["alert_id"], "alert-1");
    assert_eq!(projected["isin"], "US0378331005");
    assert_eq!(projected["removed"], true);
}

#[test]
fn project_broker_remove_crypto_price_alert_response_maps_minimal_ack() {
    let response = json!({
        "removeCryptoPriceAlert": {
            "id": "alert-1"
        }
    });

    let projected = project_broker_remove_crypto_price_alert_response("alert-1", "BTC", &response)
        .expect("project");
    assert_eq!(projected["action"], "remove");
    assert_eq!(projected["kind"], "crypto");
    assert_eq!(projected["alert_id"], "alert-1");
    assert_eq!(projected["ticker"], "BTC");
    assert_eq!(projected["removed"], true);
}

#[test]
fn project_broker_remove_price_alert_response_accepts_nonmatching_backend_id() {
    let response = json!({
        "removePriceAlert": {
            "id": "other"
        }
    });

    let projected =
        project_broker_remove_price_alert_response("alert-1", "US0378331005", &response)
            .expect("project");
    assert_eq!(projected["alert_id"], "alert-1");
    assert_eq!(projected["isin"], "US0378331005");
    assert_eq!(projected["removed"], true);
}
