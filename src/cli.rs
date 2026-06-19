use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::helpers::{broker_transactions_status_help, broker_transactions_type_filter_help};

#[derive(Debug, Parser)]
#[command(
    name = "sc",
    about = "Scalable Capital CLI",
    long_about = "Scalable Capital CLI",
    version,
    after_help = "Examples:\n  sc installation-code\n  sc login\n  sc broker context select --portfolio-id <PORTFOLIO_ID>\n  sc broker overview"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Commands {
    #[command(about = "Generate a local installation proof code")]
    InstallationCode(InstallationCodeArgs),
    #[command(about = "Authenticate and save a session")]
    Login(LoginArgs),
    #[command(about = "Remove a saved session")]
    Logout(LogoutArgs),
    #[command(about = "Run the built-in WhoAmI query")]
    Whoami(WhoamiArgs),
    #[command(about = "Broker commands")]
    Broker(BrokerArgs),
    #[command(about = "List CLI capabilities")]
    Capabilities(CapabilitiesArgs),
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(
        long,
        help = "Store the session in locally enforced read-only mode for GraphQL mutations"
    )]
    pub local_read_only: bool,
}

#[derive(Debug, Args)]
pub struct InstallationCodeArgs {
    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LogoutArgs {
    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct WhoamiArgs {
    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerArgs {
    #[command(subcommand)]
    pub command: BrokerCommand,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum BrokerCommand {
    #[command(about = "Show or persist broker account/portfolio context")]
    Context(BrokerContextArgs),
    #[command(about = "Get broker portfolio overview/valuation")]
    Overview(BrokerOverviewArgs),
    #[command(about = "Explore broker portfolio analytics")]
    Analytics(BrokerAnalyticsArgs),
    #[command(
        about = "Get broker cash, buying-power, credit, and derivatives-availability breakdown"
    )]
    CashBreakdown(BrokerCashBreakdownArgs),
    #[command(about = "List broker transactions with filters and pagination")]
    Transactions(BrokerTransactionsArgs),
    #[command(about = "Get broker transaction details by transaction id")]
    Transaction(BrokerTransactionArgs),
    #[command(about = "Get broker portfolio holdings")]
    Holdings(BrokerHoldingsArgs),
    #[command(about = "List, add, or remove broker portfolio watchlist entries")]
    Watchlist(BrokerWatchlistArgs),
    #[command(about = "Search securities within broker portfolio context")]
    Search(BrokerSearchArgs),
    #[command(about = "Discover derivatives for a known underlying ISIN")]
    Derivatives(BrokerDerivativesArgs),
    #[command(about = "Get the current quote for a security ISIN")]
    Quote(BrokerQuoteArgs),
    #[command(about = "Get news summary for a security ISIN")]
    SecurityNews(BrokerSecurityNewsArgs),
    #[command(about = "List, add, or remove broker price alerts")]
    PriceAlerts(BrokerPriceAlertsArgs),
    #[command(about = "List or add broker savings plans")]
    SavingsPlans(BrokerSavingsPlansArgs),
    #[command(about = "Trade commands")]
    Trade(BrokerTradeArgs),
}

#[derive(Debug, Args)]
pub struct BrokerContextArgs {
    #[command(subcommand)]
    pub command: BrokerContextCommand,
}

#[derive(Debug, Subcommand)]
pub enum BrokerContextCommand {
    #[command(about = "Show current broker context")]
    Show(BrokerContextShowArgs),
    #[command(about = "Set the active broker context")]
    Select(BrokerContextSelectArgs),
}

#[derive(Debug, Args)]
pub struct BrokerContextShowArgs {
    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerContextSelectArgs {
    #[arg(long, help = "Broker portfolio id")]
    pub portfolio_id: String,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerOverviewArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Include year-to-date performance points")]
    pub include_year_to_date: bool,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerAnalyticsArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerCashBreakdownArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerTransactionsArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(
        long,
        default_value_t = 20,
        value_parser = clap::value_parser!(u16).range(1..=100),
        help = "Number of transactions per page (1..100)"
    )]
    pub page_size: u16,

    #[arg(long, help = "Pagination cursor from a previous response")]
    pub cursor: Option<String>,

    #[arg(
        long = "type-filter",
        value_name = "FILTER",
        help = broker_transactions_type_filter_help()
    )]
    pub type_filter: Vec<String>,

    #[arg(
        long = "status",
        value_name = "STATUS",
        help = broker_transactions_status_help()
    )]
    pub status: Vec<String>,

    #[arg(long, help = "Optional free-text transaction search term")]
    pub search_term: Option<String>,

    #[arg(long, help = "From timestamp in ISO-8601 format")]
    pub from_time: Option<String>,

    #[arg(long, help = "To timestamp in ISO-8601 format")]
    pub to_time: Option<String>,

    #[arg(long, help = "Optional security ISIN filter")]
    pub isin: Option<String>,

    #[arg(long, help = "Include reinvestment subtypes")]
    pub include_reinvestment_subtypes: bool,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerTransactionArgs {
    #[command(subcommand)]
    pub command: BrokerTransactionCommand,
}

#[derive(Debug, Subcommand)]
pub enum BrokerTransactionCommand {
    #[command(about = "Show broker transaction details by transaction id")]
    Details(BrokerTransactionDetailsArgs),
}

#[derive(Debug, Args)]
pub struct BrokerTransactionDetailsArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Broker transaction id")]
    pub transaction_id: String,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerHoldingsArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Include year-to-date performance points for quote ticks")]
    pub include_year_to_date: bool,

    #[arg(long, help = "Optional market data source (for example CONSOLIDATED)")]
    pub quote_source: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerWatchlistArgs {
    #[command(subcommand)]
    pub command: Option<BrokerWatchlistCommand>,

    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Include year-to-date performance points for quote ticks")]
    pub include_year_to_date: bool,

    #[arg(long, help = "Optional market data source (for example CONSOLIDATED)")]
    pub quote_source: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum BrokerWatchlistCommand {
    #[command(about = "Add a security ISIN to the broker watchlist")]
    Add(BrokerWatchlistAddArgs),
    #[command(about = "Remove a security ISIN from the broker watchlist")]
    Remove(BrokerWatchlistRemoveArgs),
}

#[derive(Debug, Args)]
pub struct BrokerWatchlistAddArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Security ISIN")]
    pub isin: String,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerWatchlistRemoveArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Security ISIN")]
    pub isin: String,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerSearchArgs {
    #[arg(help = "Search query text")]
    pub query: String,

    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Include year-to-date performance points for quote ticks")]
    pub include_year_to_date: bool,

    #[arg(long, help = "Optional market data source (for example CONSOLIDATED)")]
    pub quote_source: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerDerivativesArgs {
    #[command(subcommand)]
    pub command: BrokerDerivativesCommand,
}

#[derive(Debug, Subcommand)]
pub enum BrokerDerivativesCommand {
    #[command(about = "Search derivatives for a known underlying ISIN")]
    Search(BrokerDerivativesSearchArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerDerivativeType {
    Knockout,
    Warrant,
    Factor,
}

impl BrokerDerivativeType {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Knockout => "knockout",
            Self::Warrant => "warrant",
            Self::Factor => "factor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerDerivativeStrategy {
    Long,
    Short,
    Put,
    Call,
}

impl BrokerDerivativeStrategy {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::Long => "LONG",
            Self::Short => "SHORT",
            Self::Put => "PUT",
            Self::Call => "CALL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerDerivativeIssuer {
    GoldmanSachs,
    Hsbc,
    Hvb,
    Bnp,
    Vontobel,
    MorganStanley,
    SocGen,
}

impl BrokerDerivativeIssuer {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::GoldmanSachs => "GOLDMAN_SACHS",
            Self::Hsbc => "HSBC",
            Self::Hvb => "HVB",
            Self::Bnp => "BNP",
            Self::Vontobel => "VONTOBEL",
            Self::MorganStanley => "MORGAN_STANLEY",
            Self::SocGen => "SOC_GEN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerDerivativeKnockoutSubcategory {
    MiniFuture,
    Turbo,
}

impl BrokerDerivativeKnockoutSubcategory {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::MiniFuture => "MINI_FUTURE",
            Self::Turbo => "TURBO",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerDerivativeSortField {
    Strike,
    Leverage,
    ExpiryDate,
    KnockoutBarrier,
    DistanceToKnockout,
    PremiumAbsolute,
    PremiumRelative,
    DistanceToStrike,
    Omega,
    Delta,
    ImpliedVolatility,
    Factor,
}

impl BrokerDerivativeSortField {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::Strike => "STRIKE",
            Self::Leverage => "LEVERAGE",
            Self::ExpiryDate => "EXPIRY_DATE",
            Self::KnockoutBarrier => "KNOCKOUT_BARRIER",
            Self::DistanceToKnockout => "DISTANCE_TO_KNOCKOUT",
            Self::PremiumAbsolute => "PREMIUM_ABSOLUTE",
            Self::PremiumRelative => "PREMIUM_RELATIVE",
            Self::DistanceToStrike => "DISTANCE_TO_STRIKE",
            Self::Omega => "OMEGA",
            Self::Delta => "DELTA",
            Self::ImpliedVolatility => "IMPLIED_VOLATILITY",
            Self::Factor => "FACTOR",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerDerivativeSortOrder {
    Asc,
    Desc,
}

impl BrokerDerivativeSortOrder {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Args)]
pub struct BrokerDerivativesSearchArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Underlying security ISIN")]
    pub underlying: String,

    #[arg(long = "type", value_enum, help = "Derivative family")]
    pub derivative_type: BrokerDerivativeType,

    #[arg(
        long,
        default_value_t = 50,
        value_parser = clap::value_parser!(u16).range(1..=100),
        help = "Number of derivatives per page (1..100)"
    )]
    pub limit: u16,

    #[arg(long, default_value_t = 0, help = "Page offset")]
    pub offset: u32,

    #[arg(long = "issuer", value_enum, help = "Issuer filter (repeatable)")]
    pub issuer: Vec<BrokerDerivativeIssuer>,

    #[arg(long, value_enum, required = true, help = "Strategy filter")]
    pub strategy: BrokerDerivativeStrategy,

    #[arg(
        long = "product-subcategory",
        value_enum,
        help = "Knockout product subcategory filter (repeatable)"
    )]
    pub product_subcategory: Vec<BrokerDerivativeKnockoutSubcategory>,

    #[arg(long, help = "Minimum leverage")]
    pub leverage_min: Option<String>,

    #[arg(long, help = "Maximum leverage")]
    pub leverage_max: Option<String>,

    #[arg(long, help = "Minimum knockout barrier")]
    pub knockout_barrier_min: Option<String>,

    #[arg(long, help = "Maximum knockout barrier")]
    pub knockout_barrier_max: Option<String>,

    #[arg(long, help = "Minimum strike")]
    pub strike_min: Option<String>,

    #[arg(long, help = "Maximum strike")]
    pub strike_max: Option<String>,

    #[arg(long, help = "Minimum omega")]
    pub omega_min: Option<String>,

    #[arg(long, help = "Maximum omega")]
    pub omega_max: Option<String>,

    #[arg(long, help = "Minimum delta")]
    pub delta_min: Option<String>,

    #[arg(long, help = "Maximum delta")]
    pub delta_max: Option<String>,

    #[arg(long, help = "Minimum factor")]
    pub factor_min: Option<String>,

    #[arg(long, help = "Maximum factor")]
    pub factor_max: Option<String>,

    #[arg(long, help = "Warrant expiry start date in YYYY-MM-DD format")]
    pub expiry_from: Option<String>,

    #[arg(long, help = "Warrant expiry end date in YYYY-MM-DD format")]
    pub expiry_to: Option<String>,

    #[arg(long, value_enum, help = "Sort field")]
    pub sort_field: Option<BrokerDerivativeSortField>,

    #[arg(long, value_enum, help = "Sort order")]
    pub sort_order: Option<BrokerDerivativeSortOrder>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerQuoteArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Security ISIN")]
    pub isin: String,

    #[arg(long, help = "Include year-to-date performance points for quote ticks")]
    pub include_year_to_date: bool,

    #[arg(long, help = "Optional market data source (for example CONSOLIDATED)")]
    pub quote_source: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerSecurityNewsArgs {
    #[arg(long, help = "Security ISIN")]
    pub isin: String,

    #[arg(long, help = "Locale (for example en_DE, de_DE)")]
    pub locale: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerPriceAlertsArgs {
    #[command(subcommand)]
    pub command: Option<BrokerPriceAlertsCommand>,

    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Return only active alerts")]
    pub active_only: bool,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum BrokerPriceAlertsCommand {
    #[command(about = "Add a broker price alert for a security ISIN or crypto ticker")]
    Add(BrokerPriceAlertAddArgs),
    #[command(about = "Remove a broker price alert by alert id")]
    Remove(BrokerPriceAlertRemoveArgs),
}

#[derive(Debug, Args)]
pub struct BrokerPriceAlertAddArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(
        long,
        help = "Security ISIN",
        required_unless_present = "ticker",
        conflicts_with = "ticker"
    )]
    pub isin: Option<String>,

    #[arg(
        long,
        help = "Crypto ticker",
        required_unless_present = "isin",
        conflicts_with = "isin"
    )]
    pub ticker: Option<String>,

    #[arg(long, help = "Alert trigger price (positive decimal)")]
    pub price: String,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerPriceAlertRemoveArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Price alert id")]
    pub alert_id: String,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerSavingsPlansArgs {
    #[command(subcommand)]
    pub command: Option<BrokerSavingsPlansCommand>,

    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum BrokerSavingsPlansCommand {
    #[command(about = "Create or update a broker savings plan for a security ISIN")]
    Add(BrokerSavingsPlanAddArgs),
    #[command(about = "Remove a broker savings plan for a security ISIN")]
    Remove(BrokerSavingsPlanRemoveArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerSavingsPlanFrequency {
    Monthly,
    BiMonthly,
    Quarterly,
    SemiAnnually,
    Annually,
}

impl BrokerSavingsPlanFrequency {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::Monthly => "MONTHLY",
            Self::BiMonthly => "BI_MONTHLY",
            Self::Quarterly => "QUARTERLY",
            Self::SemiAnnually => "SEMI_ANNUALLY",
            Self::Annually => "ANNUALLY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerSavingsPlanPaymentMethod {
    ReferenceAccount,
    BuyingPowerWithReferenceAccountFallback,
}

impl BrokerSavingsPlanPaymentMethod {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::ReferenceAccount => "REFERENCE_ACCOUNT",
            Self::BuyingPowerWithReferenceAccountFallback => {
                "BUYING_POWER_WITH_REFERENCE_ACCOUNT_FALLBACK"
            }
        }
    }
}

#[derive(Debug, Args)]
pub struct BrokerSavingsPlanAddArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Security ISIN")]
    pub isin: String,

    #[arg(long, help = "Savings plan amount (positive decimal)")]
    pub amount: String,

    #[arg(long, value_enum, help = "Savings plan frequency override")]
    pub frequency: Option<BrokerSavingsPlanFrequency>,

    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=31), help = "Execution day of month (1..31) override")]
    pub day_of_month: Option<u8>,

    #[arg(long, help = "Execution year-month override in YYYY-MM format")]
    pub year_month: Option<String>,

    #[arg(long, help = "Dynamization rate (positive decimal) override")]
    pub dynamization_rate: Option<String>,

    #[arg(long, value_enum, help = "Payment method override")]
    pub payment_method: Option<BrokerSavingsPlanPaymentMethod>,

    #[arg(long, help = "Appropriateness id (optional pass-through)")]
    pub appropriateness_id: Option<String>,

    #[arg(
        long,
        help = "Acknowledged appropriateness warning version (optional pass-through)"
    )]
    pub acknowledged_appropriateness_warning_version: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerSavingsPlanRemoveArgs {
    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Security ISIN")]
    pub isin: String,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerTradeArgs {
    #[command(subcommand)]
    pub command: BrokerTradeCommand,
}

#[derive(Debug, Subcommand)]
pub enum BrokerTradeCommand {
    #[command(about = "Start BUY trade flow (checks, disclosure, and submit)")]
    Buy(BrokerTradeBuyArgs),
    #[command(about = "Start SELL trade flow (checks, disclosure, and submit)")]
    Sell(BrokerTradeSellArgs),
    #[command(
        about = "Cancel a pending broker order",
        after_help = "Note:\n  The order ID returned by sc broker trade buy or sc broker trade sell\n  is the same transaction ID shown as id in sc broker transactions."
    )]
    Cancel(BrokerTradeCancelArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BrokerTradeOrderType {
    Market,
    Limit,
    Stop,
}

#[derive(Debug, Args)]
pub struct BrokerTradeBuyArgs {
    #[arg(long, help = "Security ISIN (required for phase 1 and phase 2)")]
    pub isin: Option<String>,

    #[arg(
        long,
        help = "Intended order amount in account currency (positive decimal, required for phase 1 and phase 2)"
    )]
    pub amount: Option<String>,

    #[arg(long, value_enum, default_value_t = BrokerTradeOrderType::Market, help = "Order type")]
    pub order_type: BrokerTradeOrderType,

    #[arg(long, help = "Limit price (required for --order-type limit)")]
    pub limit_price: Option<String>,

    #[arg(long, help = "Stop price (required for --order-type stop)")]
    pub stop_price: Option<String>,

    #[arg(long, help = "Trading venue override (e.g. MUNC, XETR)")]
    pub venue: Option<String>,

    #[arg(
        long,
        help = "Phase 2 confirmation id from phase 1 preview output; run phase 1 args plus --confirm to submit"
    )]
    pub confirm: Option<String>,

    #[arg(
        long,
        requires = "confirm",
        help = "Phase 2 acknowledgement required when phase 1 marks the instrument as not suitable"
    )]
    pub accept_unsuitable: bool,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerTradeSellArgs {
    #[arg(long, help = "Security ISIN (required for phase 1 and phase 2)")]
    pub isin: Option<String>,

    #[arg(
        long,
        help = "Shares to sell (positive decimal, required for phase 1 and phase 2)"
    )]
    pub shares: Option<String>,

    #[arg(long, value_enum, default_value_t = BrokerTradeOrderType::Market, help = "Order type")]
    pub order_type: BrokerTradeOrderType,

    #[arg(long, help = "Limit price (required for --order-type limit)")]
    pub limit_price: Option<String>,

    #[arg(long, help = "Stop price (required for --order-type stop)")]
    pub stop_price: Option<String>,

    #[arg(long, help = "Trading venue override (e.g. MUNC, XETR)")]
    pub venue: Option<String>,

    #[arg(
        long,
        help = "Phase 2 confirmation id from phase 1 preview output; run phase 1 args plus --confirm to submit"
    )]
    pub confirm: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BrokerTradeCancelArgs {
    #[arg(
        long,
        help = "Order ID returned by trade placement; same transaction ID shown as id in sc broker transactions"
    )]
    pub order_id: String,

    #[arg(long, help = "Portfolio id override")]
    pub portfolio_id: Option<String>,

    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CapabilitiesArgs {
    #[arg(long, help = "Print compact JSON")]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn whoami_parses() {
        let cli = Cli::parse_from(["sc", "whoami"]);
        match cli.command {
            Commands::Whoami(WhoamiArgs { json }) => {
                assert!(!json);
            }
            _ => panic!("whoami command should parse"),
        }
    }

    #[test]
    fn installation_code_parses() {
        let cli = Cli::parse_from(["sc", "installation-code"]);
        match cli.command {
            Commands::InstallationCode(InstallationCodeArgs { json }) => {
                assert!(!json);
            }
            _ => panic!("installation-code should parse"),
        }
    }

    #[test]
    fn installation_code_json_parses() {
        let cli = Cli::parse_from(["sc", "installation-code", "--json"]);
        match cli.command {
            Commands::InstallationCode(InstallationCodeArgs { json }) => {
                assert!(json);
            }
            _ => panic!("installation-code --json should parse"),
        }
    }

    #[test]
    fn login_local_read_only_parses() {
        let cli = Cli::parse_from(["sc", "login", "--local-read-only"]);
        match cli.command {
            Commands::Login(LoginArgs { local_read_only }) => {
                assert!(local_read_only);
            }
            _ => panic!("login --local-read-only should parse"),
        }
    }

    #[test]
    fn top_level_help_shows_broker_commands() {
        let mut cmd = Cli::command();
        let mut help = Vec::new();
        cmd.write_long_help(&mut help).unwrap();
        let text = String::from_utf8(help).unwrap();
        assert!(text.contains("Broker commands"));
    }

    #[test]
    fn capabilities_parses() {
        let cli = Cli::parse_from(["sc", "capabilities"]);
        match cli.command {
            Commands::Capabilities(args) => {
                assert!(!args.json);
            }
            _ => panic!("capabilities should parse"),
        }
    }

    #[test]
    fn capabilities_json_parses() {
        let cli = Cli::parse_from(["sc", "capabilities", "--json"]);
        match cli.command {
            Commands::Capabilities(args) => {
                assert!(args.json);
            }
            _ => panic!("capabilities --json should parse"),
        }
    }

    #[test]
    fn broker_context_select_requires_portfolio_id() {
        let err = Cli::try_parse_from(["sc", "broker", "context", "select"]).unwrap_err();
        assert!(err.to_string().contains("--portfolio-id"));
    }

    #[test]
    fn broker_overview_parses() {
        let cli = Cli::parse_from(["sc", "broker", "overview", "--portfolio-id", "p1"]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Overview(args),
            }) => {
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
            }
            _ => panic!("broker overview should parse"),
        }
    }

    #[test]
    fn broker_analytics_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "analytics",
            "--portfolio-id",
            "p1",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Analytics(args),
            }) => {
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert!(args.json);
            }
            _ => panic!("broker analytics should parse"),
        }
    }

    #[test]
    fn broker_cash_breakdown_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "cash-breakdown",
            "--portfolio-id",
            "p1",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::CashBreakdown(args),
            }) => {
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert!(args.json);
            }
            _ => panic!("broker cash-breakdown should parse"),
        }
    }

    #[test]
    fn broker_transactions_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "transactions",
            "--portfolio-id",
            "p1",
            "--page-size",
            "50",
            "--cursor",
            "c1",
            "--type-filter",
            "buy",
            "--type-filter",
            "distribution",
            "--status",
            "filled",
            "--status",
            "settled",
            "--search-term",
            "apple",
            "--from-time",
            "2026-03-01T00:00:00Z",
            "--to-time",
            "2026-03-10T23:59:59Z",
            "--isin",
            "US0378331005",
            "--include-reinvestment-subtypes",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Transactions(args),
            }) => {
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert_eq!(args.page_size, 50);
                assert_eq!(args.cursor.as_deref(), Some("c1"));
                assert_eq!(args.type_filter, vec!["buy", "distribution"]);
                assert_eq!(args.status, vec!["filled", "settled"]);
                assert_eq!(args.search_term.as_deref(), Some("apple"));
                assert_eq!(args.from_time.as_deref(), Some("2026-03-01T00:00:00Z"));
                assert_eq!(args.to_time.as_deref(), Some("2026-03-10T23:59:59Z"));
                assert_eq!(args.isin.as_deref(), Some("US0378331005"));
                assert!(args.include_reinvestment_subtypes);
                assert!(args.json);
            }
            _ => panic!("broker transactions should parse"),
        }
    }

    #[test]
    fn broker_transactions_defaults_parses() {
        let cli = Cli::parse_from(["sc", "broker", "transactions"]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Transactions(args),
            }) => {
                assert_eq!(args.page_size, 20);
                assert!(args.cursor.is_none());
                assert!(args.type_filter.is_empty());
                assert!(args.status.is_empty());
                assert!(!args.include_reinvestment_subtypes);
                assert!(!args.json);
            }
            _ => panic!("broker transactions should parse with defaults"),
        }
    }

    #[test]
    fn broker_transaction_details_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "transaction",
            "details",
            "--portfolio-id",
            "p1",
            "--transaction-id",
            "tx-1",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Transaction(BrokerTransactionArgs {
                        command: BrokerTransactionCommand::Details(args),
                    }),
            }) => {
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert_eq!(args.transaction_id, "tx-1");
                assert!(args.json);
            }
            _ => panic!("broker transaction details should parse"),
        }
    }

    #[test]
    fn broker_transaction_details_requires_transaction_id() {
        let err = Cli::try_parse_from(["sc", "broker", "transaction", "details"]).unwrap_err();
        assert!(err.to_string().contains("--transaction-id"));
    }

    #[test]
    fn broker_watchlist_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "watchlist",
            "--portfolio-id",
            "p1",
            "--include-year-to-date",
            "--quote-source",
            "CONSOLIDATED",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Watchlist(args),
            }) => {
                assert!(args.command.is_none());
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert!(args.include_year_to_date);
                assert_eq!(args.quote_source.as_deref(), Some("CONSOLIDATED"));
                assert!(args.json);
            }
            _ => panic!("broker watchlist should parse"),
        }
    }

    #[test]
    fn broker_quote_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "quote",
            "--portfolio-id",
            "p1",
            "--isin",
            "US0378331005",
            "--include-year-to-date",
            "--quote-source",
            "CONSOLIDATED",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Quote(args),
            }) => {
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert_eq!(args.isin, "US0378331005");
                assert!(args.include_year_to_date);
                assert_eq!(args.quote_source.as_deref(), Some("CONSOLIDATED"));
                assert!(args.json);
            }
            _ => panic!("broker quote should parse"),
        }
    }

    #[test]
    fn broker_derivatives_search_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "derivatives",
            "search",
            "--portfolio-id",
            "p1",
            "--underlying",
            "US0378331005",
            "--type",
            "knockout",
            "--limit",
            "25",
            "--offset",
            "50",
            "--issuer",
            "hsbc",
            "--issuer",
            "morgan-stanley",
            "--strategy",
            "long",
            "--product-subcategory",
            "turbo",
            "--leverage-min",
            "2",
            "--leverage-max",
            "10",
            "--knockout-barrier-min",
            "180",
            "--knockout-barrier-max",
            "200",
            "--strike-min",
            "175",
            "--strike-max",
            "195",
            "--sort-field",
            "leverage",
            "--sort-order",
            "desc",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Derivatives(BrokerDerivativesArgs {
                        command: BrokerDerivativesCommand::Search(args),
                    }),
            }) => {
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert_eq!(args.underlying, "US0378331005");
                assert_eq!(args.derivative_type, BrokerDerivativeType::Knockout);
                assert_eq!(args.limit, 25);
                assert_eq!(args.offset, 50);
                assert_eq!(
                    args.issuer,
                    vec![
                        BrokerDerivativeIssuer::Hsbc,
                        BrokerDerivativeIssuer::MorganStanley
                    ]
                );
                assert_eq!(args.strategy, BrokerDerivativeStrategy::Long);
                assert_eq!(
                    args.product_subcategory,
                    vec![BrokerDerivativeKnockoutSubcategory::Turbo]
                );
                assert_eq!(args.leverage_min.as_deref(), Some("2"));
                assert_eq!(args.leverage_max.as_deref(), Some("10"));
                assert_eq!(args.knockout_barrier_min.as_deref(), Some("180"));
                assert_eq!(args.knockout_barrier_max.as_deref(), Some("200"));
                assert_eq!(args.strike_min.as_deref(), Some("175"));
                assert_eq!(args.strike_max.as_deref(), Some("195"));
                assert_eq!(args.sort_field, Some(BrokerDerivativeSortField::Leverage));
                assert_eq!(args.sort_order, Some(BrokerDerivativeSortOrder::Desc));
                assert!(args.json);
            }
            _ => panic!("broker derivatives search should parse"),
        }
    }

    #[test]
    fn broker_derivatives_search_requires_underlying_and_type() {
        let err = Cli::try_parse_from(["sc", "broker", "derivatives", "search"]).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("--underlying"));
        assert!(message.contains("--type"));
        assert!(message.contains("--strategy"));
    }

    #[test]
    fn broker_watchlist_add_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "watchlist",
            "add",
            "--portfolio-id",
            "p1",
            "--isin",
            "US0378331005",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Watchlist(args),
            }) => match args.command {
                Some(BrokerWatchlistCommand::Add(add_args)) => {
                    assert_eq!(add_args.portfolio_id.as_deref(), Some("p1"));
                    assert_eq!(add_args.isin, "US0378331005");
                    assert!(add_args.json);
                }
                _ => panic!("expected add subcommand"),
            },
            _ => panic!("broker watchlist add should parse"),
        }
    }

    #[test]
    fn broker_watchlist_remove_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "watchlist",
            "remove",
            "--portfolio-id",
            "p1",
            "--isin",
            "US0378331005",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Watchlist(args),
            }) => match args.command {
                Some(BrokerWatchlistCommand::Remove(remove_args)) => {
                    assert_eq!(remove_args.portfolio_id.as_deref(), Some("p1"));
                    assert_eq!(remove_args.isin, "US0378331005");
                    assert!(remove_args.json);
                }
                _ => panic!("expected remove subcommand"),
            },
            _ => panic!("broker watchlist remove should parse"),
        }
    }

    #[test]
    fn broker_savings_plans_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "savings-plans",
            "--portfolio-id",
            "p1",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::SavingsPlans(args),
            }) => {
                assert!(args.command.is_none());
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert!(args.json);
            }
            _ => panic!("broker savings-plans should parse"),
        }
    }

    #[test]
    fn broker_savings_plans_add_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "savings-plans",
            "add",
            "--portfolio-id",
            "p1",
            "--isin",
            "US0378331005",
            "--amount",
            "100",
            "--frequency",
            "monthly",
            "--day-of-month",
            "5",
            "--year-month",
            "2026-04",
            "--dynamization-rate",
            "1.5",
            "--payment-method",
            "reference-account",
            "--appropriateness-id",
            "app-1",
            "--acknowledged-appropriateness-warning-version",
            "v1",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::SavingsPlans(args),
            }) => match args.command {
                Some(BrokerSavingsPlansCommand::Add(add_args)) => {
                    assert_eq!(add_args.portfolio_id.as_deref(), Some("p1"));
                    assert_eq!(add_args.isin, "US0378331005");
                    assert_eq!(add_args.amount, "100");
                    assert_eq!(
                        add_args.frequency,
                        Some(BrokerSavingsPlanFrequency::Monthly)
                    );
                    assert_eq!(add_args.day_of_month, Some(5));
                    assert_eq!(add_args.year_month.as_deref(), Some("2026-04"));
                    assert_eq!(add_args.dynamization_rate.as_deref(), Some("1.5"));
                    assert_eq!(
                        add_args.payment_method,
                        Some(BrokerSavingsPlanPaymentMethod::ReferenceAccount)
                    );
                    assert_eq!(add_args.appropriateness_id.as_deref(), Some("app-1"));
                    assert_eq!(
                        add_args
                            .acknowledged_appropriateness_warning_version
                            .as_deref(),
                        Some("v1")
                    );
                    assert!(add_args.json);
                }
                _ => panic!("expected add subcommand"),
            },
            _ => panic!("broker savings-plans add should parse"),
        }
    }

    #[test]
    fn broker_savings_plans_remove_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "savings-plans",
            "remove",
            "--portfolio-id",
            "p1",
            "--isin",
            "US0378331005",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::SavingsPlans(args),
            }) => match args.command {
                Some(BrokerSavingsPlansCommand::Remove(remove_args)) => {
                    assert_eq!(remove_args.portfolio_id.as_deref(), Some("p1"));
                    assert_eq!(remove_args.isin, "US0378331005");
                    assert!(remove_args.json);
                }
                _ => panic!("expected remove subcommand"),
            },
            _ => panic!("broker savings-plans remove should parse"),
        }
    }

    #[test]
    fn broker_trade_buy_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "500",
            "--order-type",
            "market",
            "--venue",
            "MUNC",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Buy(args),
                    }),
            }) => {
                assert_eq!(args.isin.as_deref(), Some("US0378331005"));
                assert_eq!(args.amount.as_deref(), Some("500"));
                assert_eq!(args.venue.as_deref(), Some("MUNC"));
                assert_eq!(args.order_type, BrokerTradeOrderType::Market);
                assert_eq!(args.limit_price, None);
                assert_eq!(args.stop_price, None);
                assert_eq!(args.confirm, None);
                assert!(!args.accept_unsuitable);
            }
            _ => panic!("broker trade buy should parse"),
        }
    }

    #[test]
    fn broker_trade_buy_limit_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "500",
            "--order-type",
            "limit",
            "--limit-price",
            "123.45",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Buy(args),
                    }),
            }) => {
                assert_eq!(args.order_type, BrokerTradeOrderType::Limit);
                assert_eq!(args.limit_price.as_deref(), Some("123.45"));
                assert_eq!(args.stop_price, None);
            }
            _ => panic!("broker trade buy limit should parse"),
        }
    }

    #[test]
    fn broker_trade_buy_stop_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "500",
            "--order-type",
            "stop",
            "--stop-price",
            "88.10",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Buy(args),
                    }),
            }) => {
                assert_eq!(args.order_type, BrokerTradeOrderType::Stop);
                assert_eq!(args.limit_price, None);
                assert_eq!(args.stop_price.as_deref(), Some("88.10"));
            }
            _ => panic!("broker trade buy stop should parse"),
        }
    }

    #[test]
    fn broker_trade_buy_confirm_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "500",
            "--venue",
            "MUNC",
            "--confirm",
            "scb1_deadbeef",
            "--accept-unsuitable",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Buy(args),
                    }),
            }) => {
                assert_eq!(args.confirm.as_deref(), Some("scb1_deadbeef"));
                assert_eq!(args.isin.as_deref(), Some("US0378331005"));
                assert_eq!(args.amount.as_deref(), Some("500"));
                assert_eq!(args.venue.as_deref(), Some("MUNC"));
                assert!(args.accept_unsuitable);
            }
            _ => panic!("broker trade buy confirm should parse"),
        }
    }

    #[test]
    fn broker_trade_buy_accept_unsuitable_requires_confirm() {
        let err = Cli::try_parse_from([
            "sc",
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "500",
            "--accept-unsuitable",
        ])
        .unwrap_err();

        assert!(
            err.to_string().contains("--confirm"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn broker_trade_sell_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "sell",
            "--isin",
            "US0378331005",
            "--shares",
            "1.25",
            "--order-type",
            "market",
            "--venue",
            "MUNC",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Sell(args),
                    }),
            }) => {
                assert_eq!(args.isin.as_deref(), Some("US0378331005"));
                assert_eq!(args.shares.as_deref(), Some("1.25"));
                assert_eq!(args.venue.as_deref(), Some("MUNC"));
                assert_eq!(args.order_type, BrokerTradeOrderType::Market);
                assert_eq!(args.limit_price, None);
                assert_eq!(args.stop_price, None);
                assert_eq!(args.confirm, None);
            }
            _ => panic!("broker trade sell should parse"),
        }
    }

    #[test]
    fn broker_trade_sell_limit_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "sell",
            "--isin",
            "US0378331005",
            "--shares",
            "1",
            "--order-type",
            "limit",
            "--limit-price",
            "123.45",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Sell(args),
                    }),
            }) => {
                assert_eq!(args.order_type, BrokerTradeOrderType::Limit);
                assert_eq!(args.limit_price.as_deref(), Some("123.45"));
                assert_eq!(args.stop_price, None);
            }
            _ => panic!("broker trade sell limit should parse"),
        }
    }

    #[test]
    fn broker_trade_sell_stop_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "sell",
            "--isin",
            "US0378331005",
            "--shares",
            "1",
            "--order-type",
            "stop",
            "--stop-price",
            "88.10",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Sell(args),
                    }),
            }) => {
                assert_eq!(args.order_type, BrokerTradeOrderType::Stop);
                assert_eq!(args.limit_price, None);
                assert_eq!(args.stop_price.as_deref(), Some("88.10"));
            }
            _ => panic!("broker trade sell stop should parse"),
        }
    }

    #[test]
    fn broker_trade_sell_confirm_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "sell",
            "--isin",
            "US0378331005",
            "--shares",
            "2",
            "--venue",
            "MUNC",
            "--confirm",
            "scb1_deadbeef",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Sell(args),
                    }),
            }) => {
                assert_eq!(args.confirm.as_deref(), Some("scb1_deadbeef"));
                assert_eq!(args.isin.as_deref(), Some("US0378331005"));
                assert_eq!(args.shares.as_deref(), Some("2"));
                assert_eq!(args.venue.as_deref(), Some("MUNC"));
            }
            _ => panic!("broker trade sell confirm should parse"),
        }
    }

    #[test]
    fn broker_trade_cancel_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "trade",
            "cancel",
            "--order-id",
            "order-1",
            "--portfolio-id",
            "p1",
            "--json",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command:
                    BrokerCommand::Trade(BrokerTradeArgs {
                        command: BrokerTradeCommand::Cancel(args),
                    }),
            }) => {
                assert_eq!(args.order_id, "order-1");
                assert_eq!(args.portfolio_id.as_deref(), Some("p1"));
                assert!(args.json);
            }
            _ => panic!("broker trade cancel should parse"),
        }
    }

    #[test]
    fn broker_trade_cancel_requires_order_id() {
        let err = Cli::try_parse_from(["sc", "broker", "trade", "cancel"]).unwrap_err();
        assert!(
            err.to_string().contains("--order-id"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn broker_price_alerts_add_security_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "price-alerts",
            "add",
            "--isin",
            "US0378331005",
            "--price",
            "123.45",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::PriceAlerts(args),
            }) => match args.command {
                Some(BrokerPriceAlertsCommand::Add(add_args)) => {
                    assert_eq!(add_args.isin.as_deref(), Some("US0378331005"));
                    assert_eq!(add_args.ticker, None);
                    assert_eq!(add_args.price, "123.45");
                }
                _ => panic!("expected add subcommand"),
            },
            _ => panic!("broker price-alerts add should parse"),
        }
    }

    #[test]
    fn broker_price_alerts_add_crypto_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "price-alerts",
            "add",
            "--ticker",
            "BTC",
            "--price",
            "123.45",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::PriceAlerts(args),
            }) => match args.command {
                Some(BrokerPriceAlertsCommand::Add(add_args)) => {
                    assert_eq!(add_args.isin, None);
                    assert_eq!(add_args.ticker.as_deref(), Some("BTC"));
                    assert_eq!(add_args.price, "123.45");
                }
                _ => panic!("expected add subcommand"),
            },
            _ => panic!("broker price-alerts add should parse"),
        }
    }

    #[test]
    fn broker_price_alerts_add_rejects_isin_and_ticker_together() {
        let err = Cli::try_parse_from([
            "sc",
            "broker",
            "price-alerts",
            "add",
            "--isin",
            "US0378331005",
            "--ticker",
            "BTC",
            "--price",
            "100",
        ])
        .unwrap_err();
        assert!(err.to_string().contains("cannot be used with"));
    }

    #[test]
    fn broker_price_alerts_remove_parses() {
        let cli = Cli::parse_from([
            "sc",
            "broker",
            "price-alerts",
            "remove",
            "--alert-id",
            "alert-1",
        ]);
        match cli.command {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::PriceAlerts(args),
            }) => match args.command {
                Some(BrokerPriceAlertsCommand::Remove(remove_args)) => {
                    assert_eq!(remove_args.alert_id, "alert-1");
                }
                _ => panic!("expected remove subcommand"),
            },
            _ => panic!("broker price-alerts remove should parse"),
        }
    }
}
