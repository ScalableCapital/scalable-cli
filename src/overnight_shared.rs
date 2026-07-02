use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OvernightOwnerKind {
    Personal,
    Junior,
}

impl OvernightOwnerKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Junior => "junior",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DiscoveredOvernightAccount {
    pub(crate) savings_account_id: String,
    pub(crate) display_name: String,
    pub(crate) owner_kind: OvernightOwnerKind,
    pub(crate) is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ResolvedOvernightSelection {
    pub(crate) savings_account_id: String,
    pub(crate) display_name: String,
    pub(crate) owner_kind: OvernightOwnerKind,
    pub(crate) is_active: bool,
    pub(crate) selection_source: &'static str,
}

pub(crate) fn resolve_overnight_selection(
    accounts: &[DiscoveredOvernightAccount],
    explicit_savings_account_id: Option<&str>,
) -> Result<ResolvedOvernightSelection> {
    let active_accounts = sorted_active_accounts(accounts);

    if let Some(explicit_id) = explicit_savings_account_id {
        let explicit_id = explicit_id.trim();
        if explicit_id.is_empty() {
            bail!("Overnight input invalid: field 'savings_account_id' must be a non-empty string");
        }

        if let Some(account) = active_accounts
            .iter()
            .find(|account| account.savings_account_id == explicit_id)
        {
            return Ok(ResolvedOvernightSelection {
                savings_account_id: account.savings_account_id.clone(),
                display_name: account.display_name.clone(),
                owner_kind: account.owner_kind,
                is_active: account.is_active,
                selection_source: "explicit",
            });
        }

        let available = format_candidates(&active_accounts);
        bail!(
            "Unable to resolve overnight savings account id: `{explicit_id}` is not accessible. Available active overnight accounts: {available}"
        );
    }

    match active_accounts.len() {
        0 => bail!(
            "Unable to resolve overnight savings account id: no accessible overnight accounts found."
        ),
        1 => {
            let account = &active_accounts[0];
            Ok(ResolvedOvernightSelection {
                savings_account_id: account.savings_account_id.clone(),
                display_name: account.display_name.clone(),
                owner_kind: account.owner_kind,
                is_active: account.is_active,
                selection_source: "auto_resolve",
            })
        }
        _ => bail!(
            "Unable to resolve overnight savings account id: multiple active overnight accounts found [{}]. Provide --savings-account-id <ID>.",
            format_candidates(&active_accounts)
        ),
    }
}

fn sorted_active_accounts(
    accounts: &[DiscoveredOvernightAccount],
) -> Vec<DiscoveredOvernightAccount> {
    let mut active_accounts = accounts
        .iter()
        .filter(|account| account.is_active && !account.savings_account_id.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    active_accounts.sort_by(|left, right| {
        left.savings_account_id
            .cmp(&right.savings_account_id)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    active_accounts.dedup_by(|left, right| left.savings_account_id == right.savings_account_id);
    active_accounts
}

fn format_candidates(accounts: &[DiscoveredOvernightAccount]) -> String {
    if accounts.is_empty() {
        return "<none>".to_string();
    }

    accounts
        .iter()
        .map(|account| {
            format!(
                "{} ({}, {})",
                account.savings_account_id,
                account.display_name,
                account.owner_kind.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(
        savings_account_id: &str,
        display_name: &str,
        owner_kind: OvernightOwnerKind,
        is_active: bool,
    ) -> DiscoveredOvernightAccount {
        DiscoveredOvernightAccount {
            savings_account_id: savings_account_id.to_string(),
            display_name: display_name.to_string(),
            owner_kind,
            is_active,
        }
    }

    #[test]
    fn resolve_overnight_selection_auto_resolves_single_active_account() {
        let resolved = resolve_overnight_selection(
            &[account(
                "sav-1",
                "Overnight",
                OvernightOwnerKind::Personal,
                true,
            )],
            None,
        )
        .expect("selection should resolve");

        assert_eq!(resolved.savings_account_id, "sav-1");
        assert_eq!(resolved.selection_source, "auto_resolve");
    }

    #[test]
    fn resolve_overnight_selection_requires_explicit_id_for_multiple_accounts() {
        let err = resolve_overnight_selection(
            &[
                account("sav-1", "Overnight", OvernightOwnerKind::Personal, true),
                account("sav-2", "Nina", OvernightOwnerKind::Junior, true),
            ],
            None,
        )
        .expect_err("selection should fail");

        assert!(
            err.to_string()
                .contains("multiple active overnight accounts")
        );
        assert!(err.to_string().contains("sav-1"));
        assert!(err.to_string().contains("sav-2"));
    }

    #[test]
    fn resolve_overnight_selection_validates_explicit_membership() {
        let err = resolve_overnight_selection(
            &[account(
                "sav-1",
                "Overnight",
                OvernightOwnerKind::Personal,
                true,
            )],
            Some("sav-2"),
        )
        .expect_err("selection should fail");

        assert!(err.to_string().contains("`sav-2` is not accessible"));
    }

    #[test]
    fn resolve_overnight_selection_ignores_inactive_accounts() {
        let resolved = resolve_overnight_selection(
            &[
                account("sav-1", "Inactive", OvernightOwnerKind::Personal, false),
                account("sav-2", "Active", OvernightOwnerKind::Junior, true),
            ],
            None,
        )
        .expect("selection should resolve");

        assert_eq!(resolved.savings_account_id, "sav-2");
    }
}
