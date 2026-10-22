#[derive(Clone, Copy)]
pub(crate) struct TradeVenue {
    pub(crate) mic: &'static str,
    pub(crate) emic: &'static str,
    pub(crate) alias: &'static str,
    pub(crate) discovery_label: &'static str,
    execution_label: &'static str,
}

pub(crate) const VENUE_MUNC: &str = "MUNC";
pub(crate) const VENUE_XETR: &str = "XETR";
pub(crate) const VENUE_SEIX: &str = "SEIX";

pub(crate) const VENUE_LABEL_GETTEX: &str = "Börse München (gettex)";
pub(crate) const VENUE_LABEL_XETRA: &str = "Xetra";
pub(crate) const VENUE_LABEL_SEIX: &str = "European Investor Exchange (EIX)";

pub(crate) const VENUE_OVERRIDE_HELP: &str =
    "Trading venue override (gettex/MUNC, xetra/XETR, eix/SEIX)";

pub(crate) const TRADE_VENUES: [TradeVenue; 3] = [
    TradeVenue {
        mic: VENUE_MUNC,
        emic: VENUE_MUNC,
        alias: "gettex",
        discovery_label: "Gettex",
        execution_label: VENUE_LABEL_GETTEX,
    },
    TradeVenue {
        mic: VENUE_XETR,
        emic: VENUE_XETR,
        alias: "xetra",
        discovery_label: "Xetra",
        execution_label: VENUE_LABEL_XETRA,
    },
    TradeVenue {
        mic: VENUE_SEIX,
        emic: "MUC2",
        alias: "eix",
        discovery_label: "EIX",
        execution_label: VENUE_LABEL_SEIX,
    },
];

pub(crate) fn normalize_venue_override(value: Option<&str>) -> Option<String> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_uppercase();

    TRADE_VENUES
        .iter()
        .find(|venue| normalized == venue.mic || normalized.eq_ignore_ascii_case(venue.alias))
        .map(|venue| venue.mic.to_string())
        .or(Some(normalized))
}

pub(crate) fn display_venue_label(selected_venue: &str) -> String {
    let normalized = selected_venue.trim().to_ascii_uppercase();
    TRADE_VENUES
        .iter()
        .find(|venue| normalized == venue.mic)
        .map(|venue| venue.execution_label.to_string())
        .unwrap_or_else(|| selected_venue.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_documented_venue_aliases_and_mics() {
        let cases = [
            ("gettex", VENUE_MUNC),
            ("mUnC", VENUE_MUNC),
            ("xetra", VENUE_XETR),
            ("XeTr", VENUE_XETR),
            ("eix", VENUE_SEIX),
            ("sEiX", VENUE_SEIX),
        ];

        for (input, expected) in cases {
            assert_eq!(
                normalize_venue_override(Some(input)).as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn preserves_unknown_venue_in_canonical_uppercase() {
        assert_eq!(
            normalize_venue_override(Some(" otcx ")).as_deref(),
            Some("OTCX")
        );
        assert_eq!(normalize_venue_override(Some("  ")), None);
    }

    #[test]
    fn renders_known_venue_labels_and_preserves_unknown_values() {
        assert_eq!(display_venue_label(VENUE_MUNC), VENUE_LABEL_GETTEX);
        assert_eq!(display_venue_label(VENUE_XETR), VENUE_LABEL_XETRA);
        assert_eq!(display_venue_label(VENUE_SEIX), VENUE_LABEL_SEIX);
        assert_eq!(display_venue_label("OTCX"), "OTCX");
    }
}
