use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::trade::{TradeSide, evaluate_suitability};
use crate::trade_confirmation::ConfirmationPhase1Input;
use crate::trade_execution::{
    ORDER_SIDE_BUY, ORDER_SIDE_SELL, PreparedTrade, VENUE_GETTEX, VENUE_LABEL_GETTEX,
    VENUE_LABEL_SEIX, VENUE_LABEL_XETRA, VENUE_SEIX, VENUE_XETR, canonical_decimal_from_f64,
};

#[derive(Clone, Copy)]
struct PresentationSectionSpec {
    key: &'static str,
    title: &'static str,
}

#[derive(Clone, Copy)]
struct PresentationFieldSpec {
    section_key: &'static str,
    path: &'static str,
    label: &'static str,
    nullable: bool,
}

#[derive(Clone, Copy)]
struct ExAnteFieldSpec {
    path: &'static str,
    label: &'static str,
    nullable: bool,
}

#[derive(Clone, Copy)]
struct ExAnteGroupSpec {
    title: &'static str,
    fields: &'static [ExAnteFieldSpec],
    optional: bool,
}

pub(crate) const PRESENTATION_MAPPING_INCOMPLETE: &str = "PRESENTATION_MAPPING_INCOMPLETE";
pub(crate) const PHASE1_PRESENTATION_FORMAT: &str = "markdown_sections";
pub(crate) const PHASE1_COMPLIANCE_RULE_ID: &str = "pre_trade_full_disclosure_v1";
pub(crate) const BUY_XETRA_MARKET_DATA_NOTICE: &str =
    "Market data: Börse München (gettex). Xetra's price can be found at www.boerse-frankfurt.de";
pub(crate) const BUY_EXECUTION_INSTRUCTION_TEMPLATE: &str = "I instruct Scalable Capital to place this order for execution on {exchange_name} valid for up to 360 days.";
pub(crate) const BUY_EX_ANTE_COSTS_NOTICE: &str = "The information is based on the current buy price or the selected limit or stop price. The actual execution price may differ. Financial transaction taxes (https://de.scalable.capital/en/clean-page/financial-transaction-tax) may apply. Payments may be received which depend on factors that may only be quantifiable subsequently. The actual payments received are disclosed retrospectively.";
pub(crate) const SELL_EXECUTION_INSTRUCTION_TEMPLATE: &str = "I instruct Scalable Capital to place this order for execution on {exchange_name} valid for up to 360 days.";
pub(crate) const CLIENT_DOCUMENTS_LABEL: &str = "Client documents";
pub(crate) const CLIENT_DOCUMENTS_URL: &str = "https://de.scalable.capital/dokumente";
pub(crate) const KEY_INFORMATION_DOCUMENT_LABEL: &str = "Key information document (KID)";
pub(crate) const PRICE_WARNING_LIMIT_BUY_IMMEDIATE_EXECUTION: &str =
    "Your LIMIT is above the current price. The order will be executed immediately.";
pub(crate) const PRICE_WARNING_LIMIT_SELL_IMMEDIATE_EXECUTION: &str =
    "Your LIMIT is below the current price. The order will be executed immediately.";
pub(crate) const PRICE_WARNING_STOP_BUY_IMMEDIATE_ACTIVATION: &str =
    "Your STOP is below the current price. The order will be active immediately.";
pub(crate) const PRICE_WARNING_STOP_SELL_IMMEDIATE_ACTIVATION: &str =
    "Your STOP is above the current price. The order will be active immediately.";
const OPTIONAL_ENTRY_COSTS_RESULT_ROOT: &str = "/result/ex_ante_costs/entryCosts";
const OPTIONAL_INITIAL_YEAR_COSTS_RESULT_ROOT: &str =
    "/result/ex_ante_costs/effectOnReturn/initialYearCosts";
const OPTIONAL_FOLLOWING_YEARS_COSTS_RESULT_ROOT: &str =
    "/result/ex_ante_costs/effectOnReturn/followingYearsCosts";

#[derive(Clone, Copy)]
struct OptionalExAnteFieldPolicy {
    field_prefix: &'static str,
    root_path: &'static str,
}

const OPTIONAL_EX_ANTE_FIELD_POLICIES: [OptionalExAnteFieldPolicy; 3] = [
    OptionalExAnteFieldPolicy {
        field_prefix: "/result/ex_ante_costs/entryCosts/",
        root_path: OPTIONAL_ENTRY_COSTS_RESULT_ROOT,
    },
    OptionalExAnteFieldPolicy {
        field_prefix: "/result/ex_ante_costs/effectOnReturn/initialYearCosts/",
        root_path: OPTIONAL_INITIAL_YEAR_COSTS_RESULT_ROOT,
    },
    OptionalExAnteFieldPolicy {
        field_prefix: "/result/ex_ante_costs/effectOnReturn/followingYearsCosts/",
        root_path: OPTIONAL_FOLLOWING_YEARS_COSTS_RESULT_ROOT,
    },
];

const PHASE1_REQUIRED_JSON_PATHS_BASE: [&str; 12] = [
    "/result/intent",
    "/result/market_quote",
    "/result/calculation",
    "/result/tradability",
    "/result/appropriateness",
    "/result/warning",
    "/result/price_warnings",
    "/result/ex_ante_costs",
    "/result/suitability",
    "/result/document_links",
    "/confirmation/id",
    "/confirmation/expires_at_epoch",
];

const PHASE1_PRESENTATION_SECTIONS_BASE: [PresentationSectionSpec; 11] = [
    PresentationSectionSpec {
        key: "trade_intent",
        title: "Trade intent",
    },
    PresentationSectionSpec {
        key: "market_quote",
        title: "Market quote",
    },
    PresentationSectionSpec {
        key: "calculation",
        title: "Calculation",
    },
    PresentationSectionSpec {
        key: "tradability",
        title: "Tradability",
    },
    PresentationSectionSpec {
        key: "appropriateness",
        title: "Appropriateness",
    },
    PresentationSectionSpec {
        key: "warning",
        title: "Warning",
    },
    PresentationSectionSpec {
        key: "price_warnings",
        title: "Price warnings",
    },
    PresentationSectionSpec {
        key: "ex_ante_costs",
        title: "Ex-ante costs",
    },
    PresentationSectionSpec {
        key: "suitability",
        title: "Suitability",
    },
    PresentationSectionSpec {
        key: "document_links",
        title: "Document links",
    },
    PresentationSectionSpec {
        key: "confirmation",
        title: "Confirmation",
    },
];

const PHASE1_PRESENTATION_SECTION_REGULATORY_DISCLOSURES: PresentationSectionSpec =
    PresentationSectionSpec {
        key: "regulatory_disclosures",
        title: "Regulatory disclosures",
    };

const PHASE1_PRESENTATION_FIELDS_PREFIX: [PresentationFieldSpec; 30] = [
    PresentationFieldSpec {
        section_key: "trade_intent",
        path: "/result/intent/isin",
        label: "ISIN",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "trade_intent",
        path: "/result/intent/amount",
        label: "Amount",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "trade_intent",
        path: "/result/intent/order_type",
        label: "Order type",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "trade_intent",
        path: "/result/intent/venue_override",
        label: "Venue override",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "trade_intent",
        path: "/result/intent/locale",
        label: "Locale",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "market_quote",
        path: "/result/market_quote/mid_price",
        label: "Mid price",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "market_quote",
        path: "/result/market_quote/ask_price",
        label: "Ask price",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "market_quote",
        path: "/result/market_quote/bid_price",
        label: "Bid price",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "market_quote",
        path: "/result/market_quote/currency",
        label: "Currency",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "market_quote",
        path: "/result/market_quote/is_outdated",
        label: "Is outdated",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "market_quote",
        path: "/result/market_quote/timestamp_utc",
        label: "Timestamp UTC",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "calculation",
        path: "/result/calculation/shares",
        label: "Shares",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "calculation",
        path: "/result/calculation/estimated_order_volume_raw",
        label: "Estimated order volume raw",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "calculation",
        path: "/result/calculation/estimated_order_volume",
        label: "Estimated order volume",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "tradability",
        path: "/result/tradability/status",
        label: "Status",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "tradability",
        path: "/result/tradability/selected_venue",
        label: "Selected venue",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "tradability",
        path: "/result/tradability/selected_venue_label",
        label: "Selected venue label",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "tradability",
        path: "/result/tradability/selected_venue_status",
        label: "Selected venue status",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "tradability",
        path: "/result/tradability/selected_venue_unavailability_reason",
        label: "Selected venue unavailability reason",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "tradability",
        path: "/result/tradability/requires_appropriateness",
        label: "Requires appropriateness",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "tradability",
        path: "/result/tradability/tradable",
        label: "Tradable",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "appropriateness",
        path: "/result/appropriateness/status",
        label: "Status",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "appropriateness",
        path: "/result/appropriateness/requires_warning_ack",
        label: "Requires warning acknowledgement",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "appropriateness",
        path: "/result/appropriateness/requires_questionnaire",
        label: "Requires questionnaire",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "appropriateness",
        path: "/result/appropriateness/passed",
        label: "Passed",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "appropriateness",
        path: "/result/appropriateness/appropriateness_id",
        label: "Appropriateness id",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "warning",
        path: "/result/warning/version",
        label: "Version",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "warning",
        path: "/result/warning/locale",
        label: "Locale",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "warning",
        path: "/result/warning/prompt_text",
        label: "Prompt text",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "warning",
        path: "/result/warning/acknowledgement_text",
        label: "Acknowledgement text",
        nullable: true,
    },
];

const EX_ANTE_ID_FIELDS: [ExAnteFieldSpec; 1] = [ExAnteFieldSpec {
    path: "/result/ex_ante_costs/id",
    label: "ID",
    nullable: false,
}];

const EX_ANTE_ENTRY_COSTS_FIELDS: [ExAnteFieldSpec; 6] = [
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/entryCosts/serviceCosts/amount",
        label: "Entry service amount",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/entryCosts/serviceCosts/percentage",
        label: "Entry service percentage (raw fraction, 1 = 100%)",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/entryCosts/productCosts/amount",
        label: "Entry product amount",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/entryCosts/productCosts/percentage",
        label: "Entry product percentage (raw fraction, 1 = 100%)",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/entryCosts/total/amount",
        label: "Entry total amount",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/entryCosts/total/percentage",
        label: "Entry total percentage (raw fraction, 1 = 100%)",
        nullable: true,
    },
];

const EX_ANTE_ONGOING_COSTS_FIELDS: [ExAnteFieldSpec; 6] = [
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/ongoingCosts/serviceCosts/amount",
        label: "Ongoing service amount",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/ongoingCosts/serviceCosts/percentage",
        label: "Ongoing service percentage (raw fraction, 1 = 100%)",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/ongoingCosts/productCosts/amount",
        label: "Ongoing product amount",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/ongoingCosts/productCosts/percentage",
        label: "Ongoing product percentage (raw fraction, 1 = 100%)",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/ongoingCosts/total/amount",
        label: "Ongoing total amount",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/ongoingCosts/total/percentage",
        label: "Ongoing total percentage (raw fraction, 1 = 100%)",
        nullable: false,
    },
];

const EX_ANTE_EXIT_COSTS_FIELDS: [ExAnteFieldSpec; 6] = [
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/exitCosts/serviceCosts/amount",
        label: "Exit service amount",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/exitCosts/serviceCosts/percentage",
        label: "Exit service percentage (raw fraction, 1 = 100%)",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/exitCosts/productCosts/amount",
        label: "Exit product amount",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/exitCosts/productCosts/percentage",
        label: "Exit product percentage (raw fraction, 1 = 100%)",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/exitCosts/total/amount",
        label: "Exit total amount",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/exitCosts/total/percentage",
        label: "Exit total percentage (raw fraction, 1 = 100%)",
        nullable: false,
    },
];

const EX_ANTE_EFFECT_ON_RETURN_FIELDS: [ExAnteFieldSpec; 6] = [
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/effectOnReturn/initialYearCosts/amount",
        label: "Initial year amount",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/effectOnReturn/initialYearCosts/percentage",
        label: "Initial year percentage (raw fraction, 1 = 100%)",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/effectOnReturn/followingYearsCosts/amount",
        label: "Following years amount",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/effectOnReturn/followingYearsCosts/percentage",
        label: "Following years percentage (raw fraction, 1 = 100%)",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/effectOnReturn/finalYearCosts/amount",
        label: "Final year amount",
        nullable: false,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/effectOnReturn/finalYearCosts/percentage",
        label: "Final year percentage (raw fraction, 1 = 100%)",
        nullable: false,
    },
];

const EX_ANTE_INCIDENTAL_COSTS_FIELDS: [ExAnteFieldSpec; 2] = [
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/incidentalCosts/amount",
        label: "Incidental costs amount",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/incidentalCosts/percentage",
        label: "Incidental costs percentage (raw fraction, 1 = 100%)",
        nullable: true,
    },
];

const EX_ANTE_FIVE_YEARS_COSTS_FIELDS: [ExAnteFieldSpec; 2] = [
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/fiveYearsCosts/amount",
        label: "Five years costs amount",
        nullable: true,
    },
    ExAnteFieldSpec {
        path: "/result/ex_ante_costs/fiveYearsCosts/percentage",
        label: "Five years costs percentage (raw fraction, 1 = 100%)",
        nullable: true,
    },
];

const EX_ANTE_GROUPS: [ExAnteGroupSpec; 7] = [
    ExAnteGroupSpec {
        title: "ID",
        fields: &EX_ANTE_ID_FIELDS,
        optional: false,
    },
    ExAnteGroupSpec {
        title: "Entry costs",
        fields: &EX_ANTE_ENTRY_COSTS_FIELDS,
        optional: true,
    },
    ExAnteGroupSpec {
        title: "Ongoing costs",
        fields: &EX_ANTE_ONGOING_COSTS_FIELDS,
        optional: false,
    },
    ExAnteGroupSpec {
        title: "Exit costs",
        fields: &EX_ANTE_EXIT_COSTS_FIELDS,
        optional: false,
    },
    ExAnteGroupSpec {
        title: "Effect on return",
        fields: &EX_ANTE_EFFECT_ON_RETURN_FIELDS,
        optional: false,
    },
    ExAnteGroupSpec {
        title: "Incidental costs",
        fields: &EX_ANTE_INCIDENTAL_COSTS_FIELDS,
        optional: false,
    },
    ExAnteGroupSpec {
        title: "Five years costs",
        fields: &EX_ANTE_FIVE_YEARS_COSTS_FIELDS,
        optional: false,
    },
];

const PHASE1_PRESENTATION_FIELDS_CONFIRMATION: [PresentationFieldSpec; 2] = [
    PresentationFieldSpec {
        section_key: "confirmation",
        path: "/confirmation/id",
        label: "Confirmation ID",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "confirmation",
        path: "/confirmation/expires_at_epoch",
        label: "Confirmation expiry (epoch)",
        nullable: false,
    },
];

const PHASE1_PRESENTATION_FIELDS_SUITABILITY: [PresentationFieldSpec; 4] = [
    PresentationFieldSpec {
        section_key: "suitability",
        path: "/result/suitability/status",
        label: "Status",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "suitability",
        path: "/result/suitability/is_suitable",
        label: "Suitable",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "suitability",
        path: "/result/suitability/requires_accept_unsuitable",
        label: "Requires unsuitable acknowledgement",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "suitability",
        path: "/result/suitability/accept_flag",
        label: "Phase 2 acknowledgement flag",
        nullable: true,
    },
];

const PHASE1_PRESENTATION_FIELDS_PRICE_WARNINGS: [PresentationFieldSpec; 1] =
    [PresentationFieldSpec {
        section_key: "price_warnings",
        path: "/result/price_warnings/items",
        label: "Items",
        nullable: false,
    }];

const PHASE1_PRESENTATION_FIELDS_REGULATORY_DISCLOSURES_BUY: [PresentationFieldSpec; 3] = [
    PresentationFieldSpec {
        section_key: "regulatory_disclosures",
        path: "/result/regulatory_disclosures/market_data_notice",
        label: "Market data notice",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "regulatory_disclosures",
        path: "/result/regulatory_disclosures/execution_instruction",
        label: "Execution instruction",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "regulatory_disclosures",
        path: "/result/regulatory_disclosures/ex_ante_costs_notice",
        label: "Ex-ante costs notice",
        nullable: false,
    },
];

const PHASE1_PRESENTATION_FIELDS_REGULATORY_DISCLOSURES_SELL: [PresentationFieldSpec; 1] =
    [PresentationFieldSpec {
        section_key: "regulatory_disclosures",
        path: "/result/regulatory_disclosures/execution_instruction",
        label: "Execution instruction",
        nullable: false,
    }];

const PHASE1_PRESENTATION_FIELDS_DOCUMENT_LINKS: [PresentationFieldSpec; 3] = [
    PresentationFieldSpec {
        section_key: "document_links",
        path: "/result/document_links/client_documents",
        label: "Client documents",
        nullable: false,
    },
    PresentationFieldSpec {
        section_key: "document_links",
        path: "/result/document_links/primary_kid",
        label: "Primary KID",
        nullable: true,
    },
    PresentationFieldSpec {
        section_key: "document_links",
        path: "/result/document_links/secondary_kid",
        label: "Secondary KID",
        nullable: true,
    },
];

pub(crate) struct RegulatoryDisclosures {
    pub(crate) market_data_notice: Option<&'static str>,
    pub(crate) execution_instruction: String,
    pub(crate) ex_ante_costs_notice: Option<&'static str>,
}

pub(crate) fn display_venue_label(selected_venue: &str) -> String {
    match selected_venue.trim().to_uppercase().as_str() {
        VENUE_GETTEX => VENUE_LABEL_GETTEX.to_string(),
        VENUE_XETR => VENUE_LABEL_XETRA.to_string(),
        VENUE_SEIX => VENUE_LABEL_SEIX.to_string(),
        _ => selected_venue.trim().to_string(),
    }
}

pub(crate) fn build_buy_regulatory_disclosures(
    selected_venue: &str,
    selected_venue_label: &str,
) -> RegulatoryDisclosures {
    let market_data_notice = matches!(selected_venue.trim().to_uppercase().as_str(), VENUE_XETR)
        .then_some(BUY_XETRA_MARKET_DATA_NOTICE);

    RegulatoryDisclosures {
        market_data_notice,
        execution_instruction: BUY_EXECUTION_INSTRUCTION_TEMPLATE
            .replace("{exchange_name}", selected_venue_label),
        ex_ante_costs_notice: Some(BUY_EX_ANTE_COSTS_NOTICE),
    }
}

pub(crate) fn build_sell_regulatory_disclosures(
    _selected_venue: &str,
    selected_venue_label: &str,
) -> RegulatoryDisclosures {
    RegulatoryDisclosures {
        market_data_notice: None,
        execution_instruction: SELL_EXECUTION_INSTRUCTION_TEMPLATE
            .replace("{exchange_name}", selected_venue_label),
        ex_ante_costs_notice: None,
    }
}

fn build_document_link(label: &str, url: Option<&str>) -> Value {
    match url {
        Some(url) => json!({
            "label": label,
            "url": url,
        }),
        None => Value::Null,
    }
}

fn build_document_links(prepared: &PreparedTrade) -> Value {
    json!({
        "client_documents": {
            "label": CLIENT_DOCUMENTS_LABEL,
            "url": CLIENT_DOCUMENTS_URL,
        },
        "primary_kid": build_document_link(
            KEY_INFORMATION_DOCUMENT_LABEL,
            prepared.primary_kid_url.as_deref(),
        ),
        "secondary_kid": build_document_link(
            KEY_INFORMATION_DOCUMENT_LABEL,
            prepared.secondary_kid_url.as_deref(),
        ),
    })
}

fn build_price_warning_item(
    code: &str,
    message: &str,
    trigger_field: &str,
    reference_price_type: &str,
    reference_price: &str,
) -> Value {
    json!({
        "code": code,
        "message": message,
        "trigger_field": trigger_field,
        "reference_price_type": reference_price_type,
        "reference_price": reference_price,
    })
}

fn build_price_warnings(prepared: &PreparedTrade) -> Value {
    let mut items = Vec::new();

    match prepared.intent.side {
        TradeSide::Buy => {
            if let (Some(limit_price), Some(ask_price), Some(ask_price_raw)) = (
                prepared.intent.limit_price,
                prepared.quote_ask_price_value,
                prepared.quote_ask_price.as_deref(),
            ) && prepared.intent.stop_price.is_none()
                && limit_price >= ask_price
            {
                items.push(build_price_warning_item(
                    "limit_buy_immediate_execution",
                    PRICE_WARNING_LIMIT_BUY_IMMEDIATE_EXECUTION,
                    "limit_price",
                    "ask",
                    ask_price_raw,
                ));
            }

            if let Some(stop_price) = prepared.intent.stop_price
                && stop_price <= prepared.quote_mid_price_value
            {
                items.push(build_price_warning_item(
                    "stop_buy_immediate_activation",
                    PRICE_WARNING_STOP_BUY_IMMEDIATE_ACTIVATION,
                    "stop_price",
                    "mid",
                    &prepared.quote_mid_price,
                ));
            }
        }
        TradeSide::Sell => {
            if let (Some(limit_price), Some(bid_price), Some(bid_price_raw)) = (
                prepared.intent.limit_price,
                prepared.quote_bid_price_value,
                prepared.quote_bid_price.as_deref(),
            ) && prepared.intent.stop_price.is_none()
                && limit_price <= bid_price
            {
                items.push(build_price_warning_item(
                    "limit_sell_immediate_execution",
                    PRICE_WARNING_LIMIT_SELL_IMMEDIATE_EXECUTION,
                    "limit_price",
                    "bid",
                    bid_price_raw,
                ));
            }

            if let Some(stop_price) = prepared.intent.stop_price
                && stop_price >= prepared.quote_mid_price_value
            {
                items.push(build_price_warning_item(
                    "stop_sell_immediate_activation",
                    PRICE_WARNING_STOP_SELL_IMMEDIATE_ACTIVATION,
                    "stop_price",
                    "mid",
                    &prepared.quote_mid_price,
                ));
            }
        }
    }

    json!({ "items": items })
}

pub(crate) fn build_phase2_command_template(
    confirmation_id: &str,
    phase1_input: &ConfirmationPhase1Input,
    requires_accept_unsuitable: bool,
    json_mode: bool,
) -> String {
    let mut cmd = format!(
        "sc broker trade {} --isin {} --order-type {}",
        phase1_input.side, phase1_input.isin, phase1_input.order_type
    );
    match phase1_input.side.as_str() {
        ORDER_SIDE_BUY => {
            if let Some(amount) = phase1_input.amount.as_deref() {
                cmd.push_str(&format!(" --amount {amount}"));
            }
        }
        ORDER_SIDE_SELL => {
            if let Some(shares) = phase1_input.shares.as_deref() {
                cmd.push_str(&format!(" --shares {shares}"));
            }
        }
        _ => {}
    }
    if let Some(venue) = phase1_input.venue.as_deref() {
        cmd.push_str(&format!(" --venue {venue}"));
    }
    if let Some(limit_price) = phase1_input.limit_price.as_deref() {
        cmd.push_str(&format!(" --limit-price {limit_price}"));
    }
    if let Some(stop_price) = phase1_input.stop_price.as_deref() {
        cmd.push_str(&format!(" --stop-price {stop_price}"));
    }
    cmd.push_str(&format!(" --confirm {confirmation_id}"));
    if requires_accept_unsuitable {
        cmd.push_str(" --accept-unsuitable");
    }
    if json_mode {
        cmd.push_str(" --json");
    }
    cmd
}

pub(crate) fn build_phase1_command_template(
    phase1_input: &ConfirmationPhase1Input,
    json_mode: bool,
) -> String {
    let mut cmd = format!(
        "sc broker trade {} --isin {} --order-type {}",
        phase1_input.side, phase1_input.isin, phase1_input.order_type
    );
    match phase1_input.side.as_str() {
        ORDER_SIDE_BUY => {
            if let Some(amount) = phase1_input.amount.as_deref() {
                cmd.push_str(&format!(" --amount {amount}"));
            }
        }
        ORDER_SIDE_SELL => {
            if let Some(shares) = phase1_input.shares.as_deref() {
                cmd.push_str(&format!(" --shares {shares}"));
            }
        }
        _ => {}
    }
    if let Some(venue) = phase1_input.venue.as_deref() {
        cmd.push_str(&format!(" --venue {venue}"));
    }
    if let Some(limit_price) = phase1_input.limit_price.as_deref() {
        cmd.push_str(&format!(" --limit-price {limit_price}"));
    }
    if let Some(stop_price) = phase1_input.stop_price.as_deref() {
        cmd.push_str(&format!(" --stop-price {stop_price}"));
    }
    if json_mode {
        cmd.push_str(" --json");
    }
    cmd
}

pub(crate) fn build_result_payload(
    prepared: &PreparedTrade,
    order_submission: Option<Value>,
) -> Value {
    let regulatory_disclosures =
        if order_submission.is_none() && matches!(prepared.intent.side, TradeSide::Buy) {
            let disclosures = build_buy_regulatory_disclosures(
                &prepared.tradability_gate.selected_venue,
                &prepared.selected_venue_label,
            );
            Some(json!({
                "market_data_notice": disclosures.market_data_notice,
                "execution_instruction": disclosures.execution_instruction,
                "ex_ante_costs_notice": disclosures.ex_ante_costs_notice,
            }))
        } else if order_submission.is_none() && matches!(prepared.intent.side, TradeSide::Sell) {
            let disclosures = build_sell_regulatory_disclosures(
                &prepared.tradability_gate.selected_venue,
                &prepared.selected_venue_label,
            );
            Some(json!({
                "execution_instruction": disclosures.execution_instruction,
            }))
        } else {
            None
        };
    let price_warnings = order_submission
        .is_none()
        .then(|| build_price_warnings(prepared));
    let document_links = order_submission
        .is_none()
        .then(|| build_document_links(prepared));

    let order_submission = order_submission.unwrap_or_else(|| {
        json!({
            "submitted": false,
            "reason": "phase_1_preview_only"
        })
    });
    let suitability = {
        let review = evaluate_suitability(
            prepared
                .appropriateness_json
                .get("status")
                .and_then(Value::as_str),
        );
        json!({
            "status": review.status,
            "is_suitable": review.is_suitable,
            "requires_accept_unsuitable": review.requires_accept_unsuitable,
            "accept_flag": review.requires_accept_unsuitable.then_some("--accept-unsuitable"),
        })
    };
    let mut result = json!({
        "intent": {
            "side": prepared.intent.side_label(),
            "isin": &prepared.intent.isin,
            "amount": &prepared.intent.amount_str,
            "shares": &prepared.intent.shares_str,
            "order_type": &prepared.intent.order_type,
            "limit_price": &prepared.intent.limit_price_str,
            "stop_price": &prepared.intent.stop_price_str,
            "venue_override": &prepared.intent.venue_override,
            "locale": &prepared.intent.locale,
        },
        "market_quote": {
            "mid_price": &prepared.quote_mid_price,
            "ask_price": &prepared.quote_ask_price,
            "bid_price": &prepared.quote_bid_price,
            "currency": &prepared.confirmation_fields.currency,
            "is_outdated": false,
            "timestamp_utc": &prepared.quote_timestamp_utc,
        },
        "calculation": {
            "sizing_price_basis": prepared.sizing_price_basis,
            "sizing_price": &prepared.sizing_price,
            "estimate_price_basis": prepared.estimate_price_basis,
            "estimate_price": &prepared.estimate_price,
            "shares": &prepared.number_of_shares_str,
            "estimated_order_volume_raw": canonical_decimal_from_f64(prepared.estimated_order_volume_raw),
            "estimated_order_volume": canonical_decimal_from_f64(prepared.estimated_order_volume),
            "is_whole_position_sold": prepared.is_whole_position_sold,
        },
        "tradability": {
            "status": &prepared.tradability_gate.status,
            "selected_venue": &prepared.tradability_gate.selected_venue,
            "selected_venue_label": &prepared.selected_venue_label,
            "selected_venue_status": &prepared.tradability_gate.selected_venue_status,
            "selected_venue_unavailability_reason": &prepared.tradability_gate.selected_venue_unavailability_reason,
            "requires_appropriateness": prepared.tradability_gate.requires_appropriateness,
            "tradable": prepared.tradability_gate.tradable,
            "selected_venue_sellable": prepared.tradability_gate.selected_venue_sellable,
        },
        "appropriateness": &prepared.appropriateness_json,
        "warning": &prepared.warning_json,
        "price_warnings": price_warnings.unwrap_or_else(|| json!({ "items": [] })),
        "ex_ante_costs": &prepared.ex_ante_costs,
        "suitability": suitability,
        "order_submission": order_submission,
        "pre_trade_checks_passed": true,
    });
    if let Some(regulatory_disclosures) = regulatory_disclosures {
        result["regulatory_disclosures"] = regulatory_disclosures;
    }
    if let Some(document_links) = document_links {
        result["document_links"] = document_links;
    }
    result
}

pub(crate) fn phase1_required_json_paths(side: TradeSide) -> Vec<&'static str> {
    let mut paths = PHASE1_REQUIRED_JSON_PATHS_BASE.to_vec();
    if matches!(side, TradeSide::Buy | TradeSide::Sell) {
        paths.push("/result/regulatory_disclosures");
    }
    paths
}

pub(crate) fn phase1_presentation_sections(side: TradeSide) -> Vec<&'static str> {
    phase1_sections(side)
        .iter()
        .map(|section| section.key)
        .collect()
}

fn phase1_sections(side: TradeSide) -> Vec<PresentationSectionSpec> {
    let mut sections = PHASE1_PRESENTATION_SECTIONS_BASE.to_vec();
    if matches!(side, TradeSide::Buy | TradeSide::Sell) {
        let document_links_index = sections
            .iter()
            .position(|section| section.key == "document_links")
            .unwrap_or(sections.len());
        sections.insert(
            document_links_index,
            PHASE1_PRESENTATION_SECTION_REGULATORY_DISCLOSURES,
        );
    }
    sections
}

fn phase1_fields(side: TradeSide) -> Vec<PresentationFieldSpec> {
    let ex_ante_field_count: usize = EX_ANTE_GROUPS.iter().map(|group| group.fields.len()).sum();
    let document_link_field_count = PHASE1_PRESENTATION_FIELDS_DOCUMENT_LINKS.len();
    let regulatory_field_count = if matches!(side, TradeSide::Buy) {
        PHASE1_PRESENTATION_FIELDS_REGULATORY_DISCLOSURES_BUY.len()
    } else if matches!(side, TradeSide::Sell) {
        PHASE1_PRESENTATION_FIELDS_REGULATORY_DISCLOSURES_SELL.len()
    } else {
        0
    };
    let mut fields = Vec::with_capacity(
        PHASE1_PRESENTATION_FIELDS_PREFIX.len()
            + PHASE1_PRESENTATION_FIELDS_PRICE_WARNINGS.len()
            + ex_ante_field_count
            + PHASE1_PRESENTATION_FIELDS_SUITABILITY.len()
            + regulatory_field_count
            + document_link_field_count
            + PHASE1_PRESENTATION_FIELDS_CONFIRMATION.len(),
    );
    fields.extend_from_slice(&PHASE1_PRESENTATION_FIELDS_PREFIX);
    fields.extend_from_slice(&PHASE1_PRESENTATION_FIELDS_PRICE_WARNINGS);
    fields.extend(ex_ante_presentation_fields());
    fields.extend_from_slice(&PHASE1_PRESENTATION_FIELDS_SUITABILITY);
    if matches!(side, TradeSide::Buy) {
        fields.extend_from_slice(&PHASE1_PRESENTATION_FIELDS_REGULATORY_DISCLOSURES_BUY);
    } else if matches!(side, TradeSide::Sell) {
        fields.extend_from_slice(&PHASE1_PRESENTATION_FIELDS_REGULATORY_DISCLOSURES_SELL);
    }
    fields.extend_from_slice(&PHASE1_PRESENTATION_FIELDS_DOCUMENT_LINKS);
    fields.extend_from_slice(&PHASE1_PRESENTATION_FIELDS_CONFIRMATION);
    fields
}

fn ex_ante_presentation_fields() -> Vec<PresentationFieldSpec> {
    let mut fields = Vec::new();
    for group in EX_ANTE_GROUPS {
        for field in group.fields {
            fields.push(PresentationFieldSpec {
                section_key: "ex_ante_costs",
                path: field.path,
                label: field.label,
                nullable: field.nullable,
            });
        }
    }
    fields
}

pub(crate) fn presentation_section_order_keys(side: TradeSide) -> Vec<&'static str> {
    phase1_presentation_sections(side)
}

pub(crate) fn presentation_required_leaf_paths(side: TradeSide) -> Vec<&'static str> {
    phase1_fields(side).iter().map(|field| field.path).collect()
}

fn presentation_required_leaf_paths_for_root(side: TradeSide, root: &Value) -> Vec<&'static str> {
    phase1_fields(side)
        .iter()
        .filter(|field| {
            if should_omit_optional_nullable_presentation_field(root, field) {
                return false;
            }
            true
        })
        .map(|field| field.path)
        .collect()
}

fn presentation_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn pad_decimal_string(raw: &str, min_fraction_digits: usize) -> String {
    if min_fraction_digits == 0 || raw.contains(['e', 'E']) {
        return raw.to_string();
    }

    let (sign, unsigned) = match raw.strip_prefix('-') {
        Some(value) => ("-", value),
        None => ("", raw),
    };

    match unsigned.split_once('.') {
        Some((integer, fraction)) if fraction.len() < min_fraction_digits => format!(
            "{sign}{integer}.{fraction}{}",
            "0".repeat(min_fraction_digits - fraction.len())
        ),
        Some(_) => raw.to_string(),
        None => format!("{sign}{unsigned}.{}", "0".repeat(min_fraction_digits)),
    }
}

fn ex_ante_display_value(field: ExAnteFieldSpec, value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "null".to_string(),
        Value::Number(number) if field.path.ends_with("/amount") => {
            pad_decimal_string(&number.to_string(), 2)
        }
        Value::Number(number) if field.path.ends_with("/percentage") => {
            pad_decimal_string(&number.to_string(), 5)
        }
        _ => value.to_string(),
    }
}

fn push_section_blank_line(lines: &mut Vec<String>) {
    if !lines.last().is_some_and(String::is_empty) {
        lines.push(String::new());
    }
}

fn render_ex_ante_group(payload: &Value, group: ExAnteGroupSpec) -> Option<Vec<String>> {
    let mut lines = vec![group.title.to_string()];
    for field in group.fields {
        if should_omit_optional_ex_ante_field(payload, field.path) {
            continue;
        }
        match payload.pointer(field.path) {
            Some(value) => lines.push(format!(
                "  {}: {}",
                field.label,
                ex_ante_display_value(*field, value)
            )),
            None if field.nullable => lines.push(format!("  {}: null", field.label)),
            None => lines.push(format!("  {}: <missing>", field.label)),
        }
    }

    if group.optional && lines.len() == 1 {
        return None;
    }

    Some(lines)
}

fn render_ex_ante_costs_block(payload: &Value) -> Vec<String> {
    let mut lines = vec!["Ex-ante costs".to_string()];

    for group in EX_ANTE_GROUPS {
        if let Some(group_lines) = render_ex_ante_group(payload, group) {
            lines.extend(group_lines);
        }
    }

    if let Some(ex_ante_costs_notice) = payload
        .pointer("/result/regulatory_disclosures/ex_ante_costs_notice")
        .and_then(Value::as_str)
    {
        lines.push(format!("ex_ante_costs_notice: {ex_ante_costs_notice}"));
    }

    lines
}

fn render_suitability_block(payload: &Value) -> Vec<String> {
    let mut lines = vec!["Suitability".to_string()];

    let status = payload
        .pointer("/result/suitability/status")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    let is_suitable = payload
        .pointer("/result/suitability/is_suitable")
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<missing>".to_string());
    let requires_accept_unsuitable = payload
        .pointer("/result/suitability/requires_accept_unsuitable")
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<missing>".to_string());
    let accept_flag = match payload.pointer("/result/suitability/accept_flag") {
        Some(value) if value.is_null() => "null".to_string(),
        Some(value) => value
            .as_str()
            .map(|value| value.to_string())
            .unwrap_or_else(|| value.to_string()),
        None => "<missing>".to_string(),
    };

    lines.push(format!("status: {status}"));
    lines.push(format!("is_suitable: {is_suitable}"));
    lines.push(format!(
        "requires_accept_unsuitable: {requires_accept_unsuitable}"
    ));
    lines.push(format!("phase_2_acknowledgement_flag: {accept_flag}"));

    lines
}

fn render_price_warnings_block(payload: &Value) -> Vec<String> {
    let items = payload
        .pointer("/result/price_warnings/items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Vec::new();
    }

    let mut lines = vec!["Price warnings".to_string()];
    for item in items {
        let code = item
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("<unknown_warning>");
        let message = item
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        let reference_price_type = item
            .get("reference_price_type")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        let reference_price = item
            .get("reference_price")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        lines.push(format!(
            "  {code}: {message} (reference {reference_price_type} {reference_price})"
        ));
    }

    lines
}

fn render_buy_regulatory_disclosures_lines(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(market_data_notice) = payload
        .pointer("/result/regulatory_disclosures/market_data_notice")
        .and_then(Value::as_str)
    {
        lines.push(format!("market_data_notice: {market_data_notice}"));
    }
    if let Some(execution_instruction) = payload
        .pointer("/result/regulatory_disclosures/execution_instruction")
        .and_then(Value::as_str)
    {
        lines.push(format!("execution_instruction: {execution_instruction}"));
    }

    lines
}

fn render_sell_regulatory_disclosures_lines(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(execution_instruction) = payload
        .pointer("/result/regulatory_disclosures/execution_instruction")
        .and_then(Value::as_str)
    {
        lines.push(format!("execution_instruction: {execution_instruction}"));
    }

    lines
}

fn render_document_links_block(payload: &Value) -> Vec<String> {
    let mut lines = vec!["Document links".to_string()];

    let document_links = [
        (
            "client_documents",
            "/result/document_links/client_documents",
        ),
        ("primary_kid", "/result/document_links/primary_kid"),
        ("secondary_kid", "/result/document_links/secondary_kid"),
    ];

    for (key, path) in document_links {
        let Some(value) = payload.pointer(path) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let label = value
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        let url = value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        lines.push(format!("{key}: {label} - {url}"));
    }

    if lines.len() == 1 {
        return Vec::new();
    }

    lines
}

fn should_omit_optional_nullable_presentation_field(
    root: &Value,
    spec: &PresentationFieldSpec,
) -> bool {
    spec.section_key == "ex_ante_costs" && should_omit_optional_ex_ante_field(root, spec.path)
}

fn should_omit_optional_ex_ante_field(root: &Value, path: &str) -> bool {
    optional_ex_ante_root_for_field(path)
        .is_some_and(|root_path| root.pointer(root_path).is_none_or(Value::is_null))
}

fn optional_ex_ante_root_for_field(path: &str) -> Option<&'static str> {
    OPTIONAL_EX_ANTE_FIELD_POLICIES
        .iter()
        .find(|policy| path.starts_with(policy.field_prefix))
        .map(|policy| policy.root_path)
}

pub(crate) fn build_phase1_presentation(result: &Value, confirmation: &Value) -> Result<Value> {
    let side = match result.pointer("/intent/side").and_then(Value::as_str) {
        Some(ORDER_SIDE_BUY) => TradeSide::Buy,
        Some(ORDER_SIDE_SELL) => TradeSide::Sell,
        Some(other) => bail!(
            "{PRESENTATION_MAPPING_INCOMPLETE}: unsupported trade side '{}'",
            other
        ),
        None => {
            bail!("{PRESENTATION_MAPPING_INCOMPLETE}: missing required path '/intent/side'")
        }
    };
    let mut root = Map::new();
    root.insert("result".to_string(), result.clone());
    root.insert("confirmation".to_string(), confirmation.clone());
    let root = Value::Object(root);
    let presentation_sections = phase1_sections(side);
    let presentation_fields = phase1_fields(side);
    let required_leaf_paths = presentation_required_leaf_paths_for_root(side, &root);

    let mut section_fields = std::collections::BTreeMap::<&'static str, Vec<Value>>::new();
    for section in &presentation_sections {
        section_fields.insert(section.key, Vec::new());
    }

    for spec in &presentation_fields {
        if should_omit_optional_nullable_presentation_field(&root, spec) {
            continue;
        }

        let value = match root.pointer(spec.path) {
            Some(found) => found.clone(),
            None if spec.nullable => Value::Null,
            None => bail!(
                "{PRESENTATION_MAPPING_INCOMPLETE}: missing required path '{}'",
                spec.path
            ),
        };

        let section = section_fields.get_mut(spec.section_key).ok_or_else(|| {
            anyhow!(
                "{PRESENTATION_MAPPING_INCOMPLETE}: unknown section key '{}'",
                spec.section_key
            )
        })?;
        section.push(json!({
            "path": spec.path,
            "label": spec.label,
            "value": value,
            "value_type": presentation_value_type(&value),
        }));
    }

    let mut sections = Map::new();
    for section in &presentation_sections {
        let fields = section_fields.remove(section.key).unwrap_or_default();
        sections.insert(
            section.key.to_string(),
            json!({
                "title": section.title,
                "fields": fields,
            }),
        );
    }

    Ok(json!({
        "format": PHASE1_PRESENTATION_FORMAT,
        "section_order": presentation_section_order_keys(side),
        "required_leaf_paths": required_leaf_paths,
        "sections": sections,
    }))
}

fn render_trade_prefix_lines(payload: &Value, buy: bool) -> Vec<String> {
    let isin = payload
        .get("result")
        .and_then(|v| v.get("intent"))
        .and_then(|v| v.get("isin"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let amount = payload
        .get("result")
        .and_then(|v| v.get("intent"))
        .and_then(|v| v.get("amount"))
        .and_then(Value::as_str);
    let shares = payload
        .get("result")
        .and_then(|v| v.get("calculation"))
        .and_then(|v| v.get("shares"))
        .and_then(Value::as_str);
    let sizing_price_basis = payload
        .get("result")
        .and_then(|v| v.get("calculation"))
        .and_then(|v| v.get("sizing_price_basis"))
        .and_then(Value::as_str);
    let sizing_price = payload
        .get("result")
        .and_then(|v| v.get("calculation"))
        .and_then(|v| v.get("sizing_price"))
        .and_then(Value::as_str);
    let estimate_price_basis = payload
        .get("result")
        .and_then(|v| v.get("calculation"))
        .and_then(|v| v.get("estimate_price_basis"))
        .and_then(Value::as_str);
    let estimate_price = payload
        .get("result")
        .and_then(|v| v.get("calculation"))
        .and_then(|v| v.get("estimate_price"))
        .and_then(Value::as_str);
    let selected_venue_label = payload
        .get("result")
        .and_then(|v| v.get("tradability"))
        .and_then(|v| v.get("selected_venue_label"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let currency = payload
        .get("result")
        .and_then(|v| v.get("market_quote"))
        .and_then(|v| v.get("currency"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let mid_price = payload
        .get("result")
        .and_then(|v| v.get("market_quote"))
        .and_then(|v| v.get("mid_price"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let ask_price = payload
        .get("result")
        .and_then(|v| v.get("market_quote"))
        .and_then(|v| v.get("ask_price"))
        .and_then(Value::as_str);
    let bid_price = payload
        .get("result")
        .and_then(|v| v.get("market_quote"))
        .and_then(|v| v.get("bid_price"))
        .and_then(Value::as_str);
    let order_submitted = payload
        .get("result")
        .and_then(|v| v.get("order_submission"))
        .and_then(|v| v.get("submitted"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let order_id = payload
        .get("result")
        .and_then(|v| v.get("order_submission"))
        .and_then(|v| v.get("order_id"))
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    let should_render_preview_block = payload
        .get("next_step")
        .and_then(Value::as_str)
        .map(|next_step| next_step != "completed")
        .unwrap_or(true);
    let confirmation_id = payload
        .get("confirmation")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    let next_step = payload
        .get("next_step")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let compliance_instruction = payload
        .get("compliance")
        .and_then(|v| v.get("instruction"))
        .and_then(Value::as_str);
    let mut lines = vec![format!("isin: {isin}")];
    if buy {
        if let Some(amount) = amount {
            lines.push(format!("amount: {amount}"));
        }
        if let Some(shares) = shares {
            lines.push(format!("shares: {shares}"));
        }
    } else if let Some(shares) = shares {
        lines.push(format!("shares: {shares}"));
    }
    lines.extend([
        format!("quote_mid_price: {mid_price} {currency}"),
        format!("selected_venue: {selected_venue_label}"),
        format!("order_submitted: {order_submitted}"),
        format!("order_id: {order_id}"),
    ]);
    if let Some(ask_price) = ask_price {
        lines.push(format!("quote_ask_price: {ask_price} {currency}"));
    }
    if let Some(bid_price) = bid_price {
        lines.push(format!("quote_bid_price: {bid_price} {currency}"));
    }
    if let (Some(estimate_price_basis), Some(estimate_price)) =
        (estimate_price_basis, estimate_price)
        && (estimate_price_basis != sizing_price_basis.unwrap_or_default()
            || estimate_price != sizing_price.unwrap_or_default())
    {
        lines.push(format!("estimate_price_basis: {estimate_price_basis}"));
        lines.push(format!("estimate_price: {estimate_price} {currency}"));
    }

    if should_render_preview_block {
        let price_warnings = render_price_warnings_block(payload);
        if !price_warnings.is_empty() {
            push_section_blank_line(&mut lines);
            lines.extend(price_warnings);
        }
        push_section_blank_line(&mut lines);
        lines.extend(render_ex_ante_costs_block(payload));
        push_section_blank_line(&mut lines);
        lines.extend(render_suitability_block(payload));
        if buy {
            let buy_regulatory_disclosures = render_buy_regulatory_disclosures_lines(payload);
            if !buy_regulatory_disclosures.is_empty() {
                push_section_blank_line(&mut lines);
                lines.extend(buy_regulatory_disclosures);
            }
        } else {
            let sell_regulatory_disclosures = render_sell_regulatory_disclosures_lines(payload);
            if !sell_regulatory_disclosures.is_empty() {
                push_section_blank_line(&mut lines);
                lines.extend(sell_regulatory_disclosures);
            }
        }
        let document_links = render_document_links_block(payload);
        if !document_links.is_empty() {
            push_section_blank_line(&mut lines);
            lines.extend(document_links);
        }
    }

    push_section_blank_line(&mut lines);
    lines.push(format!("confirmation_id: {confirmation_id}"));
    lines.push("pre_trade_checks_passed: true".to_string());
    lines.push(format!("next_step: {next_step}"));
    if let Some(instruction) = compliance_instruction {
        lines.push(format!("compliance_instruction: {instruction}"));
    }
    lines
}

pub(crate) fn render_trade_buy_text(payload: &Value) -> Vec<String> {
    render_trade_prefix_lines(payload, true)
}

pub(crate) fn render_trade_sell_text(payload: &Value) -> Vec<String> {
    render_trade_prefix_lines(payload, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TargetEnv;
    use crate::trade::TradeTradabilityGate;
    use crate::trade_confirmation::ConfirmationFields;
    use crate::trade_execution::{TradeIntent, VENUE_LABEL_SEIX};

    fn sample_phase1_input(side: &str) -> ConfirmationPhase1Input {
        ConfirmationPhase1Input {
            side: side.to_string(),
            isin: "DE0007100000".to_string(),
            amount: Some("500".to_string()),
            shares: Some("9".to_string()),
            venue: Some("SEIX".to_string()),
            order_type: "limit".to_string(),
            limit_price: Some("48.50".to_string()),
            stop_price: Some("47.00".to_string()),
        }
    }

    fn sample_prepared_trade(side: TradeSide) -> PreparedTrade {
        let (
            amount,
            amount_str,
            shares,
            shares_str,
            confirmation_amount,
            limit_price,
            limit_price_str,
        ) = match side {
            TradeSide::Buy => (
                Some(500.0),
                Some("500".to_string()),
                Some(9.0),
                Some("9".to_string()),
                Some("500".to_string()),
                Some(50.5),
                Some("50.50".to_string()),
            ),
            TradeSide::Sell => (
                None,
                None,
                Some(9.0),
                Some("9".to_string()),
                None,
                Some(50.9),
                Some("50.90".to_string()),
            ),
        };
        let estimate_price_value = limit_price.unwrap_or(match side {
            TradeSide::Buy => 50.5,
            TradeSide::Sell => 50.4,
        });
        let estimate_price = limit_price_str
            .clone()
            .unwrap_or_else(|| canonical_decimal_from_f64(estimate_price_value));
        PreparedTrade {
            env: TargetEnv::Prod,
            account_id: "account-1".to_string(),
            portfolio_id: "portfolio-1".to_string(),
            account_source: "context",
            portfolio_source: "context",
            intent: TradeIntent {
                side,
                isin: "DE0007100000".to_string(),
                amount,
                amount_str,
                shares,
                shares_str,
                order_type: "limit".to_string(),
                limit_price,
                stop_price: None,
                limit_price_str,
                stop_price_str: None,
                venue_override: Some("SEIX".to_string()),
                locale: "en_DE".to_string(),
            },
            tradability_gate: TradeTradabilityGate {
                status: "TRADABLE_WITHOUT_APPROPRIATENESS".to_string(),
                tradable: true,
                requires_appropriateness: false,
                selected_venue: "SEIX".to_string(),
                selected_venue_status: "TRADABLE_WITHOUT_APPROPRIATENESS".to_string(),
                selected_venue_unavailability_reason: None,
                selected_venue_sellable: Some(9.0),
            },
            appropriateness_json: json!({
                "status": "NOT_REQUIRED",
                "requires_warning_ack": false,
                "requires_questionnaire": false,
                "passed": true,
                "appropriateness_id": Value::Null
            }),
            warning_json: json!({
                "version": "v1",
                "locale": "en_DE",
                "prompt_text": "Prompt",
                "acknowledgement_text": "Ack"
            }),
            warning_version_for_order: Some("v1".to_string()),
            appropriateness_id_for_order: None,
            quote_mid_price_value: 50.4561,
            quote_ask_price_value: Some(50.5000),
            quote_bid_price_value: Some(50.4000),
            quote_mid_price: "50.4561".to_string(),
            quote_ask_price: Some("50.5000".to_string()),
            quote_bid_price: Some("50.4000".to_string()),
            quote_currency: "EUR".to_string(),
            quote_timestamp_utc: Some("2026-03-10T19:25:29.000Z".to_string()),
            selected_venue_label: VENUE_LABEL_SEIX.to_string(),
            primary_kid_url: Some("https://example.test/primary-kid.pdf".to_string()),
            secondary_kid_url: Some("https://example.test/en-kid.pdf".to_string()),
            sizing_price_basis: match side {
                TradeSide::Buy => "ask_price",
                TradeSide::Sell => "bid_price",
            },
            sizing_price: match side {
                TradeSide::Buy => "50.5000".to_string(),
                TradeSide::Sell => "50.4000".to_string(),
            },
            estimate_price_basis: "limit_price",
            estimate_price,
            number_of_shares: 9.0,
            number_of_shares_str: "9".to_string(),
            is_whole_position_sold: false,
            estimated_order_volume_raw: 454.1049,
            estimated_order_volume: 454.1049,
            ex_ante_costs: json!({
                "id": "cost-id",
                "entryCosts": {
                    "serviceCosts": {"amount": 0.12, "percentage": 0.00026},
                    "productCosts": {"amount": 0.34, "percentage": 0.00075},
                    "total": {"amount": 0.46, "percentage": 0.00101}
                },
                "ongoingCosts": {
                    "serviceCosts": {"amount": 0.05, "percentage": 0.00011},
                    "productCosts": {"amount": 0.07, "percentage": 0.00015},
                    "total": {"amount": 0.12, "percentage": 0.00026}
                },
                "exitCosts": {
                    "serviceCosts": {"amount": 0.08, "percentage": 0.00018},
                    "productCosts": {"amount": 0.09, "percentage": 0.00019},
                    "total": {"amount": 0.17, "percentage": 0.00037}
                },
                "effectOnReturn": {
                    "initialYearCosts": {"amount": 0.46, "percentage": 0.00101},
                    "followingYearsCosts": {"amount": 0.12, "percentage": 0.00026},
                    "finalYearCosts": {"amount": 0.17, "percentage": 0.00037}
                },
                "incidentalCosts": {"amount": 0.03, "percentage": 0.00007},
                "fiveYearsCosts": {"amount": 2.50, "percentage": 0.00550}
            }),
            confirmation_fields: ConfirmationFields {
                isin: "DE0007100000".to_string(),
                amount: confirmation_amount,
                currency: "EUR".to_string(),
                venue: "SEIX".to_string(),
                shares: "9".to_string(),
                entry_total: "0.99".to_string(),
                ongoing_total: "0.0".to_string(),
                exit_total: "0.99".to_string(),
                five_years_total: "2.50".to_string(),
            },
            snapshot_payload: json!({}),
            intent_checksum: "checksum".to_string(),
        }
    }

    fn find_field<'a>(presentation: &'a Value, path: &str) -> Option<&'a Value> {
        presentation
            .get("sections")?
            .as_object()?
            .values()
            .find_map(|section| {
                section
                    .get("fields")
                    .and_then(Value::as_array)
                    .and_then(|fields| {
                        fields
                            .iter()
                            .find(|field| field.get("path").and_then(Value::as_str) == Some(path))
                    })
            })
    }

    #[test]
    fn phase1_command_template_places_json_at_end() {
        let template = build_phase1_command_template(&sample_phase1_input("buy"), true);

        assert_eq!(
            template,
            "sc broker trade buy --isin DE0007100000 --order-type limit --amount 500 --venue SEIX --limit-price 48.50 --stop-price 47.00 --json"
        );
    }

    #[test]
    fn phase2_command_template_includes_confirm_before_json() {
        let template =
            build_phase2_command_template("scb1_test", &sample_phase1_input("sell"), false, true);

        assert_eq!(
            template,
            "sc broker trade sell --isin DE0007100000 --order-type limit --shares 9 --venue SEIX --limit-price 48.50 --stop-price 47.00 --confirm scb1_test --json"
        );
    }

    #[test]
    fn phase2_command_template_includes_accept_unsuitable_when_required() {
        let template =
            build_phase2_command_template("scb1_test", &sample_phase1_input("buy"), true, true);

        assert_eq!(
            template,
            "sc broker trade buy --isin DE0007100000 --order-type limit --amount 500 --venue SEIX --limit-price 48.50 --stop-price 47.00 --confirm scb1_test --accept-unsuitable --json"
        );
    }

    #[test]
    fn build_result_payload_preserves_buy_disclosures_and_preview_defaults() {
        let payload = build_result_payload(&sample_prepared_trade(TradeSide::Buy), None);

        assert_eq!(
            payload.pointer("/order_submission/submitted"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            payload
                .pointer("/order_submission/reason")
                .and_then(Value::as_str),
            Some("phase_1_preview_only")
        );
        assert_eq!(
            payload
                .pointer("/regulatory_disclosures/execution_instruction")
                .and_then(Value::as_str),
            Some(
                "I instruct Scalable Capital to place this order for execution on European Investor Exchange (EIX) valid for up to 360 days."
            )
        );
        assert_eq!(
            payload
                .pointer("/regulatory_disclosures/ex_ante_costs_notice")
                .and_then(Value::as_str),
            Some(BUY_EX_ANTE_COSTS_NOTICE)
        );
        assert_eq!(
            payload
                .pointer("/price_warnings/items/0/code")
                .and_then(Value::as_str),
            Some("limit_buy_immediate_execution")
        );
        assert_eq!(
            payload
                .pointer("/price_warnings/items")
                .and_then(Value::as_array)
                .map(|items| items.len()),
            Some(1)
        );
        assert_eq!(
            payload
                .pointer("/document_links/client_documents/url")
                .and_then(Value::as_str),
            Some(CLIENT_DOCUMENTS_URL)
        );
        assert_eq!(
            payload
                .pointer("/document_links/primary_kid/url")
                .and_then(Value::as_str),
            Some("https://example.test/primary-kid.pdf")
        );
        assert_eq!(
            payload
                .pointer("/document_links/secondary_kid/url")
                .and_then(Value::as_str),
            Some("https://example.test/en-kid.pdf")
        );
        assert_eq!(
            payload.pointer("/ex_ante_costs/id").and_then(Value::as_str),
            Some("cost-id")
        );
        assert_eq!(
            payload.pointer("/ex_ante_costs/fiveYearsCosts/amount"),
            Some(&json!(2.50))
        );
        assert_eq!(
            payload
                .pointer("/suitability/status")
                .and_then(Value::as_str),
            Some("NOT_REQUIRED")
        );
        assert_eq!(
            payload.pointer("/suitability/is_suitable"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            payload
                .pointer("/calculation/sizing_price_basis")
                .and_then(Value::as_str),
            Some("ask_price")
        );
        assert_eq!(
            payload
                .pointer("/calculation/estimate_price_basis")
                .and_then(Value::as_str),
            Some("limit_price")
        );
    }

    #[test]
    fn build_result_payload_adds_sell_disclosures_and_preview_defaults() {
        let payload = build_result_payload(&sample_prepared_trade(TradeSide::Sell), None);

        assert_eq!(
            payload.pointer("/order_submission/submitted"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            payload
                .pointer("/order_submission/reason")
                .and_then(Value::as_str),
            Some("phase_1_preview_only")
        );
        assert_eq!(
            payload
                .pointer("/regulatory_disclosures/execution_instruction")
                .and_then(Value::as_str),
            Some(
                "I instruct Scalable Capital to place this order for execution on European Investor Exchange (EIX) valid for up to 360 days."
            )
        );
        assert_eq!(
            payload
                .pointer("/price_warnings/items")
                .and_then(Value::as_array)
                .map(|items| items.len()),
            Some(0)
        );
        assert_eq!(
            payload
                .pointer("/document_links/client_documents/url")
                .and_then(Value::as_str),
            Some(CLIENT_DOCUMENTS_URL)
        );
        assert_eq!(
            payload
                .pointer("/document_links/primary_kid/url")
                .and_then(Value::as_str),
            Some("https://example.test/primary-kid.pdf")
        );
        assert!(
            payload
                .pointer("/regulatory_disclosures/ex_ante_costs_notice")
                .is_none()
        );
        assert_eq!(
            payload
                .pointer("/calculation/sizing_price_basis")
                .and_then(Value::as_str),
            Some("bid_price")
        );
        assert_eq!(
            payload
                .pointer("/calculation/estimate_price_basis")
                .and_then(Value::as_str),
            Some("limit_price")
        );
    }

    #[test]
    fn render_trade_buy_text_surfaces_estimate_price_when_it_differs_from_sizing_price() {
        let payload = json!({
            "result": {
                "intent": {
                    "side": "buy",
                    "isin": "DE0007100000",
                    "amount": "500",
                    "shares": Value::Null,
                    "order_type": "limit",
                    "limit_price": "48.50",
                    "stop_price": Value::Null,
                    "venue_override": Value::Null,
                    "locale": "en_DE"
                },
                "market_quote": {
                    "mid_price": "50.4561",
                    "ask_price": "50.5000",
                    "bid_price": "50.4000",
                    "currency": "EUR",
                    "is_outdated": false,
                    "timestamp_utc": "2026-03-10T19:25:29.000Z"
                },
                "calculation": {
                    "shares": "9",
                    "sizing_price_basis": "ask_price",
                    "sizing_price": "50.5000",
                    "estimate_price_basis": "limit_price",
                    "estimate_price": "48.5000",
                    "estimated_order_volume_raw": "436.5000",
                    "estimated_order_volume": "436.5000",
                    "is_whole_position_sold": false
                },
                "tradability": {
                    "selected_venue_label": "European Investor Exchange (EIX)"
                },
                "order_submission": {
                    "submitted": false,
                    "reason": "phase_1_preview_only"
                },
                "price_warnings": { "items": [] },
                "ex_ante_costs": {
                    "id": "cost-id",
                    "entryCosts": Value::Null,
                    "ongoingCosts": {"serviceCosts": {"amount": 0, "percentage": 0}, "productCosts": {"amount": 0, "percentage": 0}, "total": {"amount": 0, "percentage": 0}},
                    "exitCosts": {"serviceCosts": {"amount": 0, "percentage": 0}, "productCosts": {"amount": 0, "percentage": 0}, "total": {"amount": 0, "percentage": 0}},
                    "effectOnReturn": {"initialYearCosts": Value::Null, "followingYearsCosts": Value::Null, "finalYearCosts": {"amount": 0, "percentage": 0}},
                    "fiveYearsCosts": {"amount": 0, "percentage": 0},
                    "incidentalCosts": {"amount": 0, "percentage": 0}
                },
                "suitability": {
                    "status": "NOT_REQUIRED",
                    "is_suitable": true,
                    "requires_accept_unsuitable": false,
                    "accept_flag": Value::Null
                }
            },
            "confirmation": { "id": "scb1_test" },
            "next_step": "confirm_with_id"
        });

        let lines = render_trade_buy_text(&payload);

        assert!(
            lines
                .iter()
                .any(|line| line == "estimate_price_basis: limit_price")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "estimate_price: 48.5000 EUR")
        );
    }

    #[test]
    fn render_trade_buy_text_omits_estimate_price_when_it_matches_sizing_price() {
        let payload = json!({
            "result": {
                "intent": {
                    "side": "buy",
                    "isin": "DE0007100000",
                    "amount": "500",
                    "shares": Value::Null,
                    "order_type": "market",
                    "limit_price": Value::Null,
                    "stop_price": Value::Null,
                    "venue_override": Value::Null,
                    "locale": "en_DE"
                },
                "market_quote": {
                    "mid_price": "50.4561",
                    "ask_price": "50.5000",
                    "bid_price": "50.4000",
                    "currency": "EUR",
                    "is_outdated": false,
                    "timestamp_utc": "2026-03-10T19:25:29.000Z"
                },
                "calculation": {
                    "shares": "9",
                    "sizing_price_basis": "ask_price",
                    "sizing_price": "50.5000",
                    "estimate_price_basis": "ask_price",
                    "estimate_price": "50.5000",
                    "estimated_order_volume_raw": "454.5000",
                    "estimated_order_volume": "454.5000",
                    "is_whole_position_sold": false
                },
                "tradability": {
                    "selected_venue_label": "European Investor Exchange (EIX)"
                },
                "order_submission": {
                    "submitted": false,
                    "reason": "phase_1_preview_only"
                },
                "price_warnings": { "items": [] },
                "ex_ante_costs": {
                    "id": "cost-id",
                    "entryCosts": Value::Null,
                    "ongoingCosts": {"serviceCosts": {"amount": 0, "percentage": 0}, "productCosts": {"amount": 0, "percentage": 0}, "total": {"amount": 0, "percentage": 0}},
                    "exitCosts": {"serviceCosts": {"amount": 0, "percentage": 0}, "productCosts": {"amount": 0, "percentage": 0}, "total": {"amount": 0, "percentage": 0}},
                    "effectOnReturn": {"initialYearCosts": Value::Null, "followingYearsCosts": Value::Null, "finalYearCosts": {"amount": 0, "percentage": 0}},
                    "fiveYearsCosts": {"amount": 0, "percentage": 0},
                    "incidentalCosts": {"amount": 0, "percentage": 0}
                },
                "suitability": {
                    "status": "NOT_REQUIRED",
                    "is_suitable": true,
                    "requires_accept_unsuitable": false,
                    "accept_flag": Value::Null
                }
            },
            "confirmation": { "id": "scb1_test" },
            "next_step": "confirm_with_id"
        });

        let lines = render_trade_buy_text(&payload);

        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("estimate_price_basis:"))
        );
        assert!(!lines.iter().any(|line| line.starts_with("estimate_price:")));
    }

    #[test]
    fn phase1_presentation_includes_full_ex_ante_costs_surface() {
        let payload = build_result_payload(&sample_prepared_trade(TradeSide::Buy), None);
        let presentation = build_phase1_presentation(
            &payload,
            &json!({
                "id": "scb1_test",
                "expires_at_epoch": 1_777_777_777i64
            }),
        )
        .expect("presentation should build");

        let id_field =
            find_field(&presentation, "/result/ex_ante_costs/id").expect("id field should exist");
        assert_eq!(id_field.get("label").and_then(Value::as_str), Some("ID"));

        let entry_service = find_field(
            &presentation,
            "/result/ex_ante_costs/entryCosts/serviceCosts/amount",
        )
        .expect("entry service field should exist");
        assert_eq!(
            entry_service.get("label").and_then(Value::as_str),
            Some("Entry service amount")
        );
        assert_eq!(entry_service.get("value"), Some(&json!(0.12)));

        let entry_product = find_field(
            &presentation,
            "/result/ex_ante_costs/entryCosts/productCosts/amount",
        )
        .expect("entry product field should exist");
        assert_eq!(
            entry_product.get("label").and_then(Value::as_str),
            Some("Entry product amount")
        );
        assert_eq!(entry_product.get("value"), Some(&json!(0.34)));

        let incidental = find_field(
            &presentation,
            "/result/ex_ante_costs/incidentalCosts/amount",
        )
        .expect("incidental costs amount field should exist");
        assert_eq!(
            incidental.get("label").and_then(Value::as_str),
            Some("Incidental costs amount")
        );
        assert_eq!(incidental.get("value"), Some(&json!(0.03)));

        let incidental_percentage = find_field(
            &presentation,
            "/result/ex_ante_costs/incidentalCosts/percentage",
        )
        .expect("incidental costs percentage field should exist");
        assert_eq!(
            incidental_percentage.get("label").and_then(Value::as_str),
            Some("Incidental costs percentage (raw fraction, 1 = 100%)")
        );
        assert_eq!(incidental_percentage.get("value"), Some(&json!(0.00007)));

        let five_years = find_field(&presentation, "/result/ex_ante_costs/fiveYearsCosts/amount")
            .expect("five years amount field should exist");
        assert_eq!(
            five_years.get("label").and_then(Value::as_str),
            Some("Five years costs amount")
        );
        assert_eq!(five_years.get("value"), Some(&json!(2.50)));

        let required_leaf_paths = presentation
            .get("required_leaf_paths")
            .and_then(Value::as_array)
            .expect("required leaf paths");
        let entry_service_idx = required_leaf_paths
            .iter()
            .position(|item| {
                item.as_str() == Some("/result/ex_ante_costs/entryCosts/serviceCosts/amount")
            })
            .expect("entry service leaf");
        let entry_product_idx = required_leaf_paths
            .iter()
            .position(|item| {
                item.as_str() == Some("/result/ex_ante_costs/entryCosts/productCosts/amount")
            })
            .expect("entry product leaf");
        let ongoing_service_idx = required_leaf_paths
            .iter()
            .position(|item| {
                item.as_str() == Some("/result/ex_ante_costs/ongoingCosts/serviceCosts/amount")
            })
            .expect("ongoing service leaf");
        let exit_service_idx = required_leaf_paths
            .iter()
            .position(|item| {
                item.as_str() == Some("/result/ex_ante_costs/exitCosts/serviceCosts/amount")
            })
            .expect("exit service leaf");
        let suitability_idx = required_leaf_paths
            .iter()
            .position(|item| item.as_str() == Some("/result/suitability/status"))
            .expect("suitability leaf");
        let confirmation_idx = required_leaf_paths
            .iter()
            .position(|item| item.as_str() == Some("/confirmation/id"))
            .expect("confirmation leaf");

        assert!(entry_service_idx < entry_product_idx);
        assert!(entry_product_idx < ongoing_service_idx);
        assert!(ongoing_service_idx < exit_service_idx);
        assert!(exit_service_idx < suitability_idx);
        assert!(exit_service_idx < confirmation_idx);
    }

    #[test]
    fn phase1_presentation_allows_nullable_entry_costs() {
        let mut prepared = sample_prepared_trade(TradeSide::Sell);
        prepared.ex_ante_costs["entryCosts"] = Value::Null;
        prepared.confirmation_fields.entry_total = "n/a".to_string();
        let payload = build_result_payload(&prepared, None);
        let presentation = build_phase1_presentation(
            &payload,
            &json!({
                "id": "scb1_test",
                "expires_at_epoch": 1_777_777_777i64
            }),
        )
        .expect("presentation should build");

        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/entryCosts/serviceCosts/amount",
            )
            .is_none()
        );

        let required_leaf_paths = presentation
            .get("required_leaf_paths")
            .and_then(Value::as_array)
            .expect("required leaf paths");
        assert!(!required_leaf_paths.iter().any(|item| {
            item.as_str() == Some("/result/ex_ante_costs/entryCosts/serviceCosts/amount")
        }));
    }

    #[test]
    fn phase1_presentation_omits_nullable_sell_effect_on_return_fields_when_null() {
        let mut prepared = sample_prepared_trade(TradeSide::Sell);
        prepared.ex_ante_costs["effectOnReturn"]["initialYearCosts"] = Value::Null;
        prepared.ex_ante_costs["effectOnReturn"]["followingYearsCosts"] = Value::Null;
        let payload = build_result_payload(&prepared, None);
        let presentation = build_phase1_presentation(
            &payload,
            &json!({
                "id": "scb1_test",
                "expires_at_epoch": 1_777_777_777i64
            }),
        )
        .expect("presentation should build");

        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/initialYearCosts/amount",
            )
            .is_none()
        );
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/followingYearsCosts/amount",
            )
            .is_none()
        );
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/finalYearCosts/amount",
            )
            .is_some()
        );

        let required_leaf_paths = presentation
            .get("required_leaf_paths")
            .and_then(Value::as_array)
            .expect("required leaf paths");
        assert!(!required_leaf_paths.iter().any(|item| {
            item.as_str() == Some("/result/ex_ante_costs/effectOnReturn/initialYearCosts/amount")
        }));
        assert!(!required_leaf_paths.iter().any(|item| {
            item.as_str() == Some("/result/ex_ante_costs/effectOnReturn/followingYearsCosts/amount")
        }));
        assert!(required_leaf_paths.iter().any(|item| {
            item.as_str() == Some("/result/ex_ante_costs/effectOnReturn/finalYearCosts/amount")
        }));
    }

    #[test]
    fn phase1_presentation_omits_nullable_buy_effect_on_return_fields_when_null() {
        let mut prepared = sample_prepared_trade(TradeSide::Buy);
        prepared.ex_ante_costs["effectOnReturn"]["initialYearCosts"] = Value::Null;
        prepared.ex_ante_costs["effectOnReturn"]["followingYearsCosts"] = Value::Null;
        let payload = build_result_payload(&prepared, None);
        let presentation = build_phase1_presentation(
            &payload,
            &json!({
                "id": "scb1_test",
                "expires_at_epoch": 1_777_777_777i64
            }),
        )
        .expect("presentation should build");

        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/initialYearCosts/amount",
            )
            .is_none()
        );
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/followingYearsCosts/amount",
            )
            .is_none()
        );
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/finalYearCosts/amount",
            )
            .is_some()
        );
    }

    #[test]
    fn render_trade_sell_text_omits_entry_cost_group_when_entry_costs_are_null() {
        let mut prepared = sample_prepared_trade(TradeSide::Sell);
        prepared.ex_ante_costs["entryCosts"] = Value::Null;
        prepared.confirmation_fields.entry_total = "n/a".to_string();
        let result = build_result_payload(&prepared, None);
        let payload = json!({
            "result": result,
            "confirmation": {
                "id": "scb1_test",
                "expires_at_epoch": 1_777_777_777i64
            },
            "next_step": "confirm"
        });

        let rendered = render_trade_sell_text(&payload).join("\n");

        assert!(rendered.contains("Ex-ante costs"));
        assert!(!rendered.contains("Entry costs"));
        assert!(rendered.contains("Ongoing costs"));
    }

    #[test]
    fn render_trade_sell_text_omits_nullable_effect_on_return_rows_when_null() {
        let mut prepared = sample_prepared_trade(TradeSide::Sell);
        prepared.ex_ante_costs["effectOnReturn"]["initialYearCosts"] = Value::Null;
        prepared.ex_ante_costs["effectOnReturn"]["followingYearsCosts"] = Value::Null;
        let result = build_result_payload(&prepared, None);
        let payload = json!({
            "result": result,
            "confirmation": {
                "id": "scb1_test",
                "expires_at_epoch": 1_777_777_777i64
            },
            "next_step": "confirm"
        });

        let rendered = render_trade_sell_text(&payload).join("\n");

        assert!(rendered.contains("Effect on return"));
        assert!(!rendered.contains("Initial year amount"));
        assert!(!rendered.contains("Following years amount"));
        assert!(rendered.contains("Final year amount"));
    }

    #[test]
    fn phase1_presentation_and_text_both_omit_absent_entry_costs() {
        let mut prepared = sample_prepared_trade(TradeSide::Buy);
        prepared.ex_ante_costs["entryCosts"] = Value::Null;
        prepared.confirmation_fields.entry_total = "n/a".to_string();
        let result = build_result_payload(&prepared, None);
        let confirmation = json!({
            "id": "scb1_test",
            "expires_at_epoch": 1_777_777_777i64
        });
        let presentation =
            build_phase1_presentation(&result, &confirmation).expect("presentation should build");
        let payload = json!({
            "result": result,
            "confirmation": confirmation,
            "next_step": "confirm"
        });

        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/entryCosts/serviceCosts/amount",
            )
            .is_none()
        );
        assert!(
            !render_trade_buy_text(&payload)
                .join("\n")
                .contains("Entry costs")
        );
    }
}
