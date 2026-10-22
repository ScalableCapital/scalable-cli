use super::*;
use crate::overnight_queries::OvernightTransactionsOptions;
use serde_json::{Value, json};

#[test]
fn project_overnight_discovery_maps_personal_and_junior_accounts() {
    let response = json!({
        "account": {
            "savingsAccounts": [
                {
                    "__typename": "OvernightSavingsAccount",
                    "id": "sav-2",
                    "personalizations": {
                        "name": "Emergency Fund"
                    },
                    "state": "ACTIVE"
                },
                {
                    "__typename": "FixedTermSavingsAccount",
                    "id": "fixed-1",
                    "personalizations": {
                        "name": "Fixed"
                    },
                    "state": "ACTIVE"
                }
            ]
        },
        "productList": {
            "minors": [
                {
                    "id": "sav-1",
                    "name": "",
                    "type": "OVERNIGHT_SAVINGS",
                    "onboardingState": "COMPLETE",
                    "state": "ACTIVE",
                    "owners": [
                        {
                            "firstName": "Nina"
                        }
                    ],
                    "productOffering": {
                        "name": "overnight"
                    }
                },
                {
                    "id": "broker-1",
                    "name": "Junior Broker",
                    "type": "BROKER",
                    "onboardingState": "COMPLETE",
                    "state": "ACTIVE",
                    "owners": [],
                    "productOffering": null
                }
            ]
        }
    });

    let projected = project_overnight_discovery_response(&response).expect("project");

    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].savings_account_id, "sav-1");
    assert_eq!(projected[0].display_name, "Nina");
    assert_eq!(projected[0].owner_kind, OvernightOwnerKind::Junior);
    assert!(projected[0].is_active);
    assert_eq!(projected[1].savings_account_id, "sav-2");
    assert_eq!(projected[1].display_name, "Emergency Fund");
    assert_eq!(projected[1].owner_kind, OvernightOwnerKind::Personal);
}

#[test]
fn project_overnight_discovery_allows_missing_minors() {
    let response = json!({
        "account": {
            "savingsAccounts": [
                {
                    "__typename": "OvernightSavingsAccount",
                    "id": "sav-2",
                    "personalizations": {
                        "name": "Emergency Fund"
                    },
                    "state": "ACTIVE"
                }
            ]
        }
    });

    let projected = project_overnight_discovery_response(&response).expect("project");

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].savings_account_id, "sav-2");
    assert_eq!(projected[0].display_name, "Emergency Fund");
    assert_eq!(projected[0].owner_kind, OvernightOwnerKind::Personal);
    assert!(projected[0].is_active);
}

#[test]
fn project_overnight_discovery_skips_malformed_personal_overnight_entries() {
    let response = json!({
        "account": {
            "savingsAccounts": [
                {
                    "__typename": "OvernightSavingsAccount",
                    "id": "sav-broken",
                    "personalizations": {
                        "name": "Broken"
                    }
                },
                {
                    "__typename": "OvernightSavingsAccount",
                    "id": "sav-2",
                    "personalizations": {
                        "name": "Emergency Fund"
                    },
                    "state": "ACTIVE"
                }
            ]
        }
    });

    let projected = project_overnight_discovery_response(&response).expect("project");

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].savings_account_id, "sav-2");
    assert_eq!(projected[0].display_name, "Emergency Fund");
    assert_eq!(projected[0].owner_kind, OvernightOwnerKind::Personal);
    assert!(projected[0].is_active);
}

#[test]
fn project_overnight_discovery_allows_junior_only_response() {
    let response = json!({
        "productList": {
            "minors": [
                {
                    "id": "sav-1",
                    "type": "OVERNIGHT_SAVINGS",
                    "onboardingState": "COMPLETE",
                    "state": "ACTIVE",
                    "owners": [
                        {
                            "firstName": "Nina"
                        }
                    ],
                    "productOffering": {
                        "name": "overnight"
                    }
                }
            ]
        }
    });

    let projected = project_overnight_discovery_response(&response).expect("project");

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].savings_account_id, "sav-1");
    assert_eq!(projected[0].display_name, "Nina");
    assert_eq!(projected[0].owner_kind, OvernightOwnerKind::Junior);
    assert!(projected[0].is_active);
}

#[test]
fn project_overnight_discovery_skips_malformed_non_overnight_minors() {
    let response = json!({
        "productList": {
            "minors": [
                {
                    "id": "broker-1",
                    "name": "Broken Broker",
                    "onboardingState": "COMPLETE",
                    "state": "ACTIVE"
                },
                {
                    "id": "sav-1",
                    "type": "OVERNIGHT_SAVINGS",
                    "onboardingState": "COMPLETE",
                    "state": "ACTIVE",
                    "owners": [
                        {
                            "firstName": "Nina"
                        }
                    ],
                    "productOffering": {
                        "name": "overnight"
                    }
                }
            ]
        }
    });

    let projected = project_overnight_discovery_response(&response).expect("project");

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].savings_account_id, "sav-1");
    assert_eq!(projected[0].display_name, "Nina");
    assert_eq!(projected[0].owner_kind, OvernightOwnerKind::Junior);
    assert!(projected[0].is_active);
}

#[test]
fn project_overnight_discovery_skips_malformed_junior_overnight_entries() {
    let response = json!({
        "productList": {
            "minors": [
                {
                    "id": "sav-broken",
                    "type": "OVERNIGHT_SAVINGS",
                    "state": "ACTIVE",
                    "owners": [
                        {
                            "firstName": "Broken"
                        }
                    ]
                },
                {
                    "id": "sav-1",
                    "type": "OVERNIGHT_SAVINGS",
                    "onboardingState": "COMPLETE",
                    "state": "ACTIVE",
                    "owners": [
                        {
                            "firstName": "Nina"
                        }
                    ],
                    "productOffering": {
                        "name": "overnight"
                    }
                }
            ]
        }
    });

    let projected = project_overnight_discovery_response(&response).expect("project");

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].savings_account_id, "sav-1");
    assert_eq!(projected[0].display_name, "Nina");
    assert_eq!(projected[0].owner_kind, OvernightOwnerKind::Junior);
    assert!(projected[0].is_active);
}

#[test]
fn project_overnight_summary_maps_expected_fields() {
    let input = OvernightSummaryInput::new("person-1", "sav-1").expect("input");
    let response = json!({
        "account": {
            "savingsAccount": {
                "id": "sav-1",
                "interests": {
                    "currentAccruedAmount": "1.23",
                    "currentInterestBearingAmount": "1000",
                    "depositAccruedLifetimeAmount": "12.34",
                    "depositInterestRate": "0.02",
                    "estimatedNextPayoutAmount": "0.98",
                    "nextPayoutDate": {
                        "epochSecond": 0
                    }
                },
                "nextPayoutDate": {
                    "epochSecond": 0
                },
                "totalAmount": "1001.23"
            }
        }
    });

    let projected = project_overnight_summary_response(&input, &response).expect("project");

    assert_eq!(projected["balance"], "1001.23");
    assert_eq!(projected["current_accrued_amount"], "1.23");
    assert_eq!(projected["current_interest_bearing_amount"], "1000");
    assert_eq!(projected["deposit_accrued_lifetime_amount"], "12.34");
    assert_eq!(projected["interest_rate"], "0.02");
    assert_eq!(projected["estimated_next_payout_amount"], "0.98");
    assert_eq!(projected["next_payout_date"], "1970-01-01T00:00:00+00:00");
    assert!(
        projected
            .get("effective_yearly_deposit_interest_rate")
            .is_none()
    );
    assert!(projected.get("granted_overdraft_interest_rate").is_none());
    assert!(projected.get("current_deposit_interest_schemes").is_none());
}

#[test]
fn project_overnight_summary_allows_missing_next_payout_date() {
    let input = OvernightSummaryInput::new("person-1", "sav-1").expect("input");
    let response = json!({
        "account": {
            "savingsAccount": {
                "id": "sav-1",
                "interests": {
                    "currentAccruedAmount": "1.23",
                    "currentInterestBearingAmount": "1000",
                    "depositAccruedLifetimeAmount": "12.34",
                    "depositInterestRate": "0.02",
                    "estimatedNextPayoutAmount": "0.98"
                },
                "totalAmount": "1001.23"
            }
        }
    });

    let projected = project_overnight_summary_response(&input, &response).expect("project");

    assert_eq!(projected["next_payout_date"], Value::Null);
}

#[test]
fn project_overnight_summary_rejects_missing_interests() {
    let input = OvernightSummaryInput::new("person-1", "sav-1").expect("input");
    let response = json!({
        "account": {
            "savingsAccount": {
                "id": "sav-1",
                "totalAmount": "1001.23"
            }
        }
    });

    let err = project_overnight_summary_response(&input, &response).expect_err("project");
    assert!(err.to_string().contains("account.savingsAccount.interests"));
}

#[test]
fn project_overnight_transactions_maps_cash_transaction_page_in_backend_order() {
    let input = overnight_transactions_input();
    let response = json!({
        "account": {
            "savingsAccount": {
                "id": "sav-1",
                "moreTransactions": {
                    "cursor": "next-page",
                    "total": 2,
                    "transactions": [
                        {
                            "id": "tx-2",
                            "currency": "EUR",
                            "type": "CASH_TRANSACTION",
                            "status": "CONFIRMED",
                            "isCancellation": false,
                            "lastEventDateTime": "2026-03-02T12:00:00Z",
                            "description": "Interest",
                            "cashTransactionType": "INTEREST",
                            "amount": "1.23",
                            "custodian": "BAADER_BANK",
                            "relatedIsin": null
                        },
                        {
                            "id": "tx-1",
                            "currency": "EUR",
                            "type": "CASH_TRANSACTION",
                            "status": "SETTLED",
                            "isCancellation": false,
                            "lastEventDateTime": null,
                            "description": "Deposit",
                            "cashTransactionType": "DEPOSIT",
                            "amount": "100.00",
                            "custodian": null,
                            "relatedIsin": null
                        }
                    ]
                }
            }
        }
    });

    let projected = project_overnight_transactions_response(&input, &response).expect("project");

    assert_eq!(projected["cursor"], "next-page");
    assert_eq!(projected["total"], 2);
    assert_eq!(projected["count"], 2);
    assert_eq!(projected["items"][0]["id"], "tx-2");
    assert_eq!(projected["items"][1]["id"], "tx-1");
    assert_eq!(projected["items"][0]["cash_transaction_type"], "INTEREST");
}

#[test]
fn project_overnight_transactions_rejects_mismatched_account_id() {
    let input = overnight_transactions_input();
    let response = json!({
        "account": {
            "savingsAccount": {
                "id": "sav-2",
                "moreTransactions": {"cursor": null, "total": 0, "transactions": []}
            }
        }
    });

    let err = project_overnight_transactions_response(&input, &response)
        .expect_err("account mismatch should fail");
    assert!(err.to_string().contains("did not match"));
}

#[test]
fn project_overnight_transactions_accepts_empty_page() {
    let input = overnight_transactions_input();
    let response = json!({
        "account": {
            "savingsAccount": {
                "id": "sav-1",
                "moreTransactions": {"cursor": null, "total": 0, "transactions": []}
            }
        }
    });

    let projected = project_overnight_transactions_response(&input, &response).expect("project");

    assert_eq!(projected["cursor"], Value::Null);
    assert_eq!(projected["total"], 0);
    assert_eq!(projected["count"], 0);
    assert_eq!(projected["items"], json!([]));
}

#[test]
fn project_overnight_transactions_rejects_malformed_page_shape() {
    let input = overnight_transactions_input();
    let malformed_responses = [
        json!({"account": {"savingsAccount": {"id": "sav-1"}}}),
        json!({"account": {"savingsAccount": {"id": "sav-1", "moreTransactions": {"total": 1, "transactions": {}}}}}),
        json!({"account": {"savingsAccount": {"id": "sav-1", "moreTransactions": {"transactions": []}}}}),
        json!({"account": {"savingsAccount": {"id": "sav-1", "moreTransactions": {"total": null, "transactions": []}}}}),
    ];

    for response in malformed_responses {
        assert!(project_overnight_transactions_response(&input, &response).is_err());
    }
}

fn overnight_transactions_input() -> OvernightTransactionsInput {
    let options =
        OvernightTransactionsOptions::new(20, None, &[], None, None, None).expect("options");
    OvernightTransactionsInput::new("person-1", "sav-1", options).expect("input")
}
