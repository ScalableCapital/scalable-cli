use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::config::{config_dir_path, set_private_file_permissions, write_private_file_atomic};
use crate::payload_fingerprint::checksum_for_payload;

const CONFIRMATION_FILE_NAME: &str = "savings_plan_confirmation.json";
const STORE_LOCK_FILE_NAME: &str = "savings_plan_confirmation.lock";
const ACTIVE_SUBMISSION_LOCK_FILE_NAME: &str = "savings_plan_active_submission.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavingsPlanConfirmation {
    pub(crate) confirmation_id: String,
    pub(crate) snapshot_checksum: String,
    pub(crate) created_at_epoch: i64,
    pub(crate) expires_at_epoch: i64,
    pub(crate) env: String,
    pub(crate) account_id: String,
    pub(crate) portfolio_id: String,
    pub(crate) isin: String,
    pub(crate) snapshot: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UnknownSavingsPlanSubmission {
    pub(crate) env: String,
    pub(crate) account_id: String,
    pub(crate) portfolio_id: String,
    pub(crate) isin: String,
    pub(crate) timestamp_epoch: i64,
    pub(crate) state: String,
}

pub(crate) struct SavingsPlanSubmissionInspection {
    pub(crate) marker: UnknownSavingsPlanSubmission,
    pub(crate) cleared_unknown_submission_gate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubmissionStarted {
    confirmation_id: String,
    env: String,
    account_id: String,
    portfolio_id: String,
    isin: String,
    started_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConsumedConfirmation {
    confirmation_id: String,
    consumed_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
enum ConfirmationState {
    Pending(SavingsPlanConfirmation),
    SubmissionStarted(SubmissionStarted),
    Unknown(UnknownSavingsPlanSubmission),
    Consumed(ConsumedConfirmation),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConfirmationStore {
    state: Option<ConfirmationState>,
}

pub(crate) struct StartedSavingsPlanSubmission {
    pub(crate) confirmation: SavingsPlanConfirmation,
    _guard: SubmissionGuard,
    _active_guard: ActiveSubmissionGuard,
}

pub(crate) fn store_pending(confirmation: SavingsPlanConfirmation) -> Result<()> {
    validate_confirmation(&confirmation)?;
    let _lock = StoreLock::acquire()?;
    let store = load_store()?;
    match store.state {
        Some(ConfirmationState::Unknown(_)) | Some(ConfirmationState::SubmissionStarted(_)) => {
            bail!(
                "SAVINGS_PLAN_SUBMISSION_UNKNOWN: inspect `sc broker savings-plans` for the matching account and portfolio before creating a new preview"
            );
        }
        Some(ConfirmationState::Pending(_)) | Some(ConfirmationState::Consumed(_)) | None => {}
    }
    save_store(&ConfirmationStore {
        state: Some(ConfirmationState::Pending(confirmation)),
    })
}

pub(crate) fn assert_preview_allowed() -> Result<()> {
    let _lock = StoreLock::acquire()?;
    let store = load_store()?;
    match store.state {
        Some(ConfirmationState::Unknown(_)) | Some(ConfirmationState::SubmissionStarted(_)) => {
            bail!(
                "SAVINGS_PLAN_SUBMISSION_UNKNOWN: inspect `sc broker savings-plans` for the matching account and portfolio before creating a new preview"
            );
        }
        Some(ConfirmationState::Pending(_)) | Some(ConfirmationState::Consumed(_)) | None => Ok(()),
    }
}

pub(crate) fn load_pending(
    confirmation_id: &str,
    now_epoch: i64,
) -> Result<SavingsPlanConfirmation> {
    let confirmation_id = normalized_confirmation_id(confirmation_id)?;
    let _lock = StoreLock::acquire()?;
    let store = load_store()?;
    match store.state {
        Some(ConfirmationState::Pending(confirmation)) => {
            if confirmation.confirmation_id != confirmation_id {
                bail!(
                    "SAVINGS_PLAN_CONFIRMATION_NOT_FOUND: no phase 1 snapshot found for id '{confirmation_id}'"
                );
            }
            if now_epoch > confirmation.expires_at_epoch {
                bail!(
                    "SAVINGS_PLAN_CONFIRMATION_EXPIRED: confirmation id '{confirmation_id}' expired at epoch {}",
                    confirmation.expires_at_epoch
                );
            }
            Ok(confirmation)
        }
        Some(ConfirmationState::Consumed(consumed))
            if consumed.confirmation_id == confirmation_id =>
        {
            bail!(
                "SAVINGS_PLAN_CONFIRMATION_ALREADY_USED: confirmation id '{confirmation_id}' was already used"
            );
        }
        Some(ConfirmationState::SubmissionStarted(started))
            if started.confirmation_id == confirmation_id =>
        {
            bail!(
                "SAVINGS_PLAN_SUBMISSION_UNKNOWN: submission for confirmation id '{confirmation_id}' has already started; inspect `sc broker savings-plans` before any new preview"
            );
        }
        Some(ConfirmationState::Unknown(_)) => bail!(
            "SAVINGS_PLAN_SUBMISSION_UNKNOWN: inspect `sc broker savings-plans` for the matching account and portfolio before creating a new preview"
        ),
        Some(ConfirmationState::Consumed(_))
        | Some(ConfirmationState::SubmissionStarted(_))
        | None => {
            bail!(
                "SAVINGS_PLAN_CONFIRMATION_NOT_FOUND: no phase 1 snapshot found for id '{confirmation_id}'"
            );
        }
    }
}

pub(crate) fn start_submission(
    confirmation_id: &str,
    expected_checksum: &str,
    now_epoch: i64,
) -> Result<StartedSavingsPlanSubmission> {
    let confirmation_id = normalized_confirmation_id(confirmation_id)?;
    let Some(guard) = SubmissionGuard::try_acquire(&confirmation_id)? else {
        mark_started_as_unknown(&confirmation_id, now_epoch)?;
        bail!(
            "SAVINGS_PLAN_SUBMISSION_UNKNOWN: another process may be submitting confirmation id '{confirmation_id}'; inspect `sc broker savings-plans`"
        );
    };
    let Some(active_guard) = ActiveSubmissionGuard::try_acquire()? else {
        mark_started_as_unknown(&confirmation_id, now_epoch)?;
        bail!(
            "SAVINGS_PLAN_SUBMISSION_UNKNOWN: another savings-plan submission may still be active; inspect `sc broker savings-plans`"
        );
    };

    let confirmation = {
        let _lock = StoreLock::acquire()?;
        let store = load_store()?;
        let Some(ConfirmationState::Pending(confirmation)) = store.state else {
            return start_state_error(&confirmation_id);
        };
        if confirmation.confirmation_id != confirmation_id {
            return start_state_error(&confirmation_id);
        }
        if confirmation.snapshot_checksum != expected_checksum {
            bail!(
                "SAVINGS_PLAN_CONFIRMATION_FIELDS_MISMATCH: confirmation id '{confirmation_id}' changed before submission"
            );
        }
        if now_epoch > confirmation.expires_at_epoch {
            bail!(
                "SAVINGS_PLAN_CONFIRMATION_EXPIRED: confirmation id '{confirmation_id}' expired at epoch {}",
                confirmation.expires_at_epoch
            );
        }
        let started = SubmissionStarted {
            confirmation_id: confirmation.confirmation_id.clone(),
            env: confirmation.env.clone(),
            account_id: confirmation.account_id.clone(),
            portfolio_id: confirmation.portfolio_id.clone(),
            isin: confirmation.isin.clone(),
            started_at_epoch: now_epoch,
        };
        save_store(&ConfirmationStore {
            state: Some(ConfirmationState::SubmissionStarted(started)),
        })?;
        confirmation
    };

    Ok(StartedSavingsPlanSubmission {
        confirmation,
        _guard: guard,
        _active_guard: active_guard,
    })
}

pub(crate) fn finalize_consumed(
    started: &StartedSavingsPlanSubmission,
    now_epoch: i64,
) -> Result<()> {
    let confirmation_id = started.confirmation.confirmation_id.clone();
    let _lock = StoreLock::acquire()?;
    let store = load_store()?;
    ensure_submission_started(&store, &confirmation_id)?;
    save_store(&ConfirmationStore {
        state: Some(ConfirmationState::Consumed(ConsumedConfirmation {
            confirmation_id,
            consumed_at_epoch: now_epoch,
        })),
    })
}

pub(crate) fn finalize_unknown(
    started: &StartedSavingsPlanSubmission,
    now_epoch: i64,
) -> Result<()> {
    let confirmation_id = started.confirmation.confirmation_id.clone();
    let marker = unknown_marker_from_confirmation(&started.confirmation, now_epoch);
    let _lock = StoreLock::acquire()?;
    let store = load_store()?;
    match store.state {
        Some(ConfirmationState::SubmissionStarted(current))
            if current.confirmation_id == confirmation_id =>
        {
            save_store(&ConfirmationStore {
                state: Some(ConfirmationState::Unknown(marker)),
            })
        }
        Some(ConfirmationState::Unknown(_)) => Ok(()),
        _ => bail!(
            "SAVINGS_PLAN_SUBMISSION_UNKNOWN: unable to prove final state for confirmation id '{confirmation_id}'"
        ),
    }
}

pub(crate) fn clear_on_logout() -> Result<()> {
    let _lock = StoreLock::acquire()?;
    let store = load_store()?;
    match store.state {
        Some(ConfirmationState::Unknown(_)) | Some(ConfirmationState::SubmissionStarted(_)) => {
            Ok(())
        }
        Some(ConfirmationState::Pending(_)) | Some(ConfirmationState::Consumed(_)) | None => {
            delete_store_file()
        }
    }
}

pub(crate) fn inspect_and_clear_matching(
    env: &str,
    account_id: &str,
    portfolio_id: &str,
) -> Result<Option<SavingsPlanSubmissionInspection>> {
    let observed = {
        let _lock = StoreLock::acquire()?;
        let store = load_store()?;
        match store.state {
            Some(ConfirmationState::Unknown(marker))
                if matches_marker(&marker, env, account_id, portfolio_id) =>
            {
                Some(InspectionCandidate::Unknown(marker))
            }
            Some(ConfirmationState::SubmissionStarted(started))
                if matches_started(&started, env, account_id, portfolio_id) =>
            {
                Some(InspectionCandidate::SubmissionStarted(started))
            }
            Some(ConfirmationState::Unknown(_))
            | Some(ConfirmationState::SubmissionStarted(_))
            | Some(ConfirmationState::Pending(_))
            | Some(ConfirmationState::Consumed(_))
            | None => None,
        }
    };

    let Some(observed) = observed else {
        return Ok(None);
    };
    match observed {
        InspectionCandidate::Unknown(marker) => {
            let Some(_active_guard) = ActiveSubmissionGuard::try_acquire()? else {
                return Ok(Some(SavingsPlanSubmissionInspection {
                    marker,
                    cleared_unknown_submission_gate: false,
                }));
            };
            let _lock = StoreLock::acquire()?;
            let store = load_store()?;
            match store.state {
                Some(ConfirmationState::Unknown(current))
                    if matches_marker(&current, env, account_id, portfolio_id) =>
                {
                    save_store(&ConfirmationStore::default())?;
                    Ok(Some(SavingsPlanSubmissionInspection {
                        marker: current,
                        cleared_unknown_submission_gate: true,
                    }))
                }
                _ => Ok(None),
            }
        }
        InspectionCandidate::SubmissionStarted(started) => {
            let Some(_guard) = SubmissionGuard::try_acquire(&started.confirmation_id)? else {
                return Ok(Some(SavingsPlanSubmissionInspection {
                    marker: unknown_marker_from_started(&started),
                    cleared_unknown_submission_gate: false,
                }));
            };
            let Some(_active_guard) = ActiveSubmissionGuard::try_acquire()? else {
                return Ok(Some(SavingsPlanSubmissionInspection {
                    marker: unknown_marker_from_started(&started),
                    cleared_unknown_submission_gate: false,
                }));
            };
            let _lock = StoreLock::acquire()?;
            let store = load_store()?;
            match store.state {
                Some(ConfirmationState::SubmissionStarted(current))
                    if current.confirmation_id == started.confirmation_id
                        && matches_started(&current, env, account_id, portfolio_id) =>
                {
                    let marker = unknown_marker_from_started(&current);
                    save_store(&ConfirmationStore::default())?;
                    Ok(Some(SavingsPlanSubmissionInspection {
                        marker,
                        cleared_unknown_submission_gate: true,
                    }))
                }
                _ => Ok(None),
            }
        }
    }
}

enum InspectionCandidate {
    Unknown(UnknownSavingsPlanSubmission),
    SubmissionStarted(SubmissionStarted),
}

fn start_state_error(confirmation_id: &str) -> Result<StartedSavingsPlanSubmission> {
    bail!(
        "SAVINGS_PLAN_CONFIRMATION_NOT_FOUND: no phase 1 snapshot found for id '{confirmation_id}'"
    )
}

fn mark_started_as_unknown(confirmation_id: &str, now_epoch: i64) -> Result<()> {
    let _lock = StoreLock::acquire()?;
    let store = load_store()?;
    let Some(ConfirmationState::SubmissionStarted(started)) = store.state else {
        return Ok(());
    };
    if started.confirmation_id != confirmation_id {
        return Ok(());
    }
    save_store(&ConfirmationStore {
        state: Some(ConfirmationState::Unknown(UnknownSavingsPlanSubmission {
            env: started.env,
            account_id: started.account_id,
            portfolio_id: started.portfolio_id,
            isin: started.isin,
            timestamp_epoch: now_epoch,
            state: "unknown".to_string(),
        })),
    })
}

fn ensure_submission_started(store: &ConfirmationStore, confirmation_id: &str) -> Result<()> {
    match &store.state {
        Some(ConfirmationState::SubmissionStarted(current))
            if current.confirmation_id == confirmation_id =>
        {
            Ok(())
        }
        _ => bail!(
            "SAVINGS_PLAN_SUBMISSION_UNKNOWN: unable to prove final state for confirmation id '{confirmation_id}'"
        ),
    }
}

fn unknown_marker_from_confirmation(
    confirmation: &SavingsPlanConfirmation,
    now_epoch: i64,
) -> UnknownSavingsPlanSubmission {
    UnknownSavingsPlanSubmission {
        env: confirmation.env.clone(),
        account_id: confirmation.account_id.clone(),
        portfolio_id: confirmation.portfolio_id.clone(),
        isin: confirmation.isin.clone(),
        timestamp_epoch: now_epoch,
        state: "unknown".to_string(),
    }
}

fn unknown_marker_from_started(started: &SubmissionStarted) -> UnknownSavingsPlanSubmission {
    UnknownSavingsPlanSubmission {
        env: started.env.clone(),
        account_id: started.account_id.clone(),
        portfolio_id: started.portfolio_id.clone(),
        isin: started.isin.clone(),
        timestamp_epoch: started.started_at_epoch,
        state: "submission_started".to_string(),
    }
}

fn matches_marker(
    marker: &UnknownSavingsPlanSubmission,
    env: &str,
    account_id: &str,
    portfolio_id: &str,
) -> bool {
    marker.env == env && marker.account_id == account_id && marker.portfolio_id == portfolio_id
}

fn matches_started(
    started: &SubmissionStarted,
    env: &str,
    account_id: &str,
    portfolio_id: &str,
) -> bool {
    started.env == env && started.account_id == account_id && started.portfolio_id == portfolio_id
}

fn validate_confirmation(confirmation: &SavingsPlanConfirmation) -> Result<()> {
    normalized_confirmation_id(&confirmation.confirmation_id)?;
    for (name, value) in [
        ("snapshot_checksum", confirmation.snapshot_checksum.as_str()),
        ("env", confirmation.env.as_str()),
        ("account_id", confirmation.account_id.as_str()),
        ("portfolio_id", confirmation.portfolio_id.as_str()),
        ("isin", confirmation.isin.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("SAVINGS_PLAN_CONFIRMATION_CORRUPT: field '{name}' must not be blank");
        }
    }
    if confirmation.expires_at_epoch < confirmation.created_at_epoch {
        bail!("SAVINGS_PLAN_CONFIRMATION_CORRUPT: expiry precedes creation time");
    }
    if checksum_for_payload(&confirmation.snapshot) != confirmation.snapshot_checksum {
        bail!(
            "SAVINGS_PLAN_CONFIRMATION_CORRUPT: snapshot checksum does not match stored snapshot"
        );
    }
    Ok(())
}

fn normalized_confirmation_id(raw: &str) -> Result<String> {
    let id = raw.trim();
    if !id.starts_with("scsp1_")
        || id.len() <= "scsp1_".len()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        bail!("SAVINGS_PLAN_CONFIRMATION_NOT_FOUND: confirmation id is invalid");
    }
    Ok(id.to_string())
}

fn load_store() -> Result<ConfirmationStore> {
    let path = confirmation_file_path()?;
    if !path.exists() {
        return Ok(ConfirmationStore::default());
    }
    let raw = fs::read_to_string(&path).with_context(|| {
        format!(
            "SAVINGS_PLAN_CONFIRMATION_CORRUPT: failed reading {}",
            path.display()
        )
    })?;
    let store: ConfirmationStore = serde_json::from_str(&raw).with_context(|| {
        format!(
            "SAVINGS_PLAN_CONFIRMATION_CORRUPT: invalid JSON at {}",
            path.display()
        )
    })?;
    if let Some(ConfirmationState::Pending(confirmation)) = &store.state {
        validate_confirmation(confirmation)?;
    }
    Ok(store)
}

fn save_store(store: &ConfirmationStore) -> Result<()> {
    let path = confirmation_file_path()?;
    let serialized = serde_json::to_string_pretty(store)?;
    write_private_file_atomic(&path, serialized.as_bytes()).with_context(|| {
        format!(
            "SAVINGS_PLAN_CONFIRMATION_CORRUPT: failed writing {}",
            path.display()
        )
    })
}

fn delete_store_file() -> Result<()> {
    let path = confirmation_file_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "SAVINGS_PLAN_CONFIRMATION_CORRUPT: failed deleting {}",
                path.display()
            )
        }),
    }
}

fn confirmation_file_path() -> Result<PathBuf> {
    Ok(config_dir_path()?.join(CONFIRMATION_FILE_NAME))
}

struct StoreLock {
    _file: File,
}

impl StoreLock {
    fn acquire() -> Result<Self> {
        let file = open_lock_file(&config_dir_path()?.join(STORE_LOCK_FILE_NAME))?;
        lock_file(&file, false)?
            .then_some(Self { _file: file })
            .ok_or_else(|| {
                anyhow!(
                    "SAVINGS_PLAN_SUBMISSION_UNKNOWN: unable to acquire confirmation-state lock"
                )
            })
    }
}

struct SubmissionGuard {
    _file: File,
}

impl SubmissionGuard {
    fn try_acquire(confirmation_id: &str) -> Result<Option<Self>> {
        let path =
            config_dir_path()?.join(format!("savings_plan_submission_{confirmation_id}.lock"));
        let file = open_lock_file(&path)?;
        Ok(lock_file(&file, true)?.then_some(Self { _file: file }))
    }
}

struct ActiveSubmissionGuard {
    _file: File,
}

impl ActiveSubmissionGuard {
    fn try_acquire() -> Result<Option<Self>> {
        let path = config_dir_path()?.join(ACTIVE_SUBMISSION_LOCK_FILE_NAME);
        let file = open_lock_file(&path)?;
        Ok(lock_file(&file, true)?.then_some(Self { _file: file }))
    }
}

fn open_lock_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("Failed opening confirmation lock {}", path.display()))?;
    set_private_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn lock_file(file: &File, nonblocking: bool) -> Result<bool> {
    use std::os::fd::AsRawFd;

    let flags = libc::LOCK_EX | if nonblocking { libc::LOCK_NB } else { 0 };
    // SAFETY: `file` remains open for the lifetime of the advisory lock and its
    // descriptor is obtained from the standard library's `AsRawFd` contract.
    let result = unsafe { libc::flock(file.as_raw_fd(), flags) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if nonblocking && error.kind() == std::io::ErrorKind::WouldBlock {
        return Ok(false);
    }
    Err(error).context("flock LOCK_EX failed")
}

#[cfg(not(unix))]
fn lock_file(_file: &File, _nonblocking: bool) -> Result<bool> {
    bail!("SAVINGS_PLAN_SUBMISSION_UNKNOWN: advisory file locks are unavailable on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct EnvGuard {
        previous: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(path: &Path) -> Self {
            let lock = crate::lock_test_env();
            let previous = std::env::var("SC_CONFIG_DIR").ok();
            unsafe { std::env::set_var("SC_CONFIG_DIR", path) };
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe { std::env::set_var("SC_CONFIG_DIR", value) },
                None => unsafe { std::env::remove_var("SC_CONFIG_DIR") },
            }
        }
    }

    fn setup() -> (TempDir, EnvGuard) {
        let temp = tempfile::tempdir().expect("temp config dir");
        let guard = EnvGuard::set(temp.path());
        (temp, guard)
    }

    #[test]
    fn config_dir_guard_holds_environment_lock_until_restoration() {
        let (temp, guard) = setup();
        let previous = guard.previous.clone();
        assert!(matches!(
            crate::test_env_mutex().try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));
        assert_eq!(
            std::env::var_os("SC_CONFIG_DIR").as_deref(),
            Some(temp.path().as_os_str())
        );

        drop(guard);
        let _lock = crate::lock_test_env();
        assert_eq!(std::env::var("SC_CONFIG_DIR").ok(), previous);
    }

    fn sample_confirmation(id: &str) -> SavingsPlanConfirmation {
        let snapshot = serde_json::json!({"costs": {"entry": null}});
        SavingsPlanConfirmation {
            confirmation_id: id.to_string(),
            snapshot_checksum: checksum_for_payload(&snapshot),
            created_at_epoch: 100,
            expires_at_epoch: 1_000,
            env: "dev".to_string(),
            account_id: "account-1".to_string(),
            portfolio_id: "portfolio-1".to_string(),
            isin: "US0378331005".to_string(),
            snapshot,
        }
    }

    #[test]
    fn pending_confirmation_round_trips_and_replaces_prior_pending_state() {
        let (_temp, _guard) = setup();
        store_pending(sample_confirmation("scsp1_first")).expect("save first");
        store_pending(sample_confirmation("scsp1_second")).expect("replace pending");

        assert!(load_pending("scsp1_first", 101).is_err());
        let loaded = load_pending("scsp1_second", 101).expect("load second");
        assert_eq!(loaded.isin, "US0378331005");
    }

    #[test]
    fn submission_started_blocks_preview_until_matching_list_inspection_after_guard_releases() {
        let (_temp, _guard) = setup();
        let confirmation = sample_confirmation("scsp1_started");
        let checksum = confirmation.snapshot_checksum.clone();
        store_pending(confirmation).expect("save pending");
        let started = start_submission("scsp1_started", checksum.as_str(), 200).expect("start");

        let live_marker = inspect_and_clear_matching("dev", "account-1", "portfolio-1")
            .expect("inspect while live")
            .expect("marker");
        assert_eq!(live_marker.marker.state, "submission_started");
        assert!(!live_marker.cleared_unknown_submission_gate);
        assert!(assert_preview_allowed().is_err());

        drop(started);
        let cleared = inspect_and_clear_matching("dev", "account-1", "portfolio-1")
            .expect("inspect after guard release")
            .expect("marker");
        assert_eq!(cleared.marker.isin, "US0378331005");
        assert!(cleared.cleared_unknown_submission_gate);
        assert_preview_allowed().expect("matching inspection clears gate");
    }

    #[test]
    fn unknown_submission_survives_logout_and_clears_only_for_matching_binding() {
        let (_temp, _guard) = setup();
        let confirmation = sample_confirmation("scsp1_unknown");
        let checksum = confirmation.snapshot_checksum.clone();
        store_pending(confirmation).expect("save pending");
        let started = start_submission("scsp1_unknown", checksum.as_str(), 200).expect("start");
        finalize_unknown(&started, 201).expect("mark unknown");

        let live_inspection = inspect_and_clear_matching("dev", "account-1", "portfolio-1")
            .expect("inspect while unknown submission remains live")
            .expect("inspection result");
        assert!(!live_inspection.cleared_unknown_submission_gate);
        assert!(assert_preview_allowed().is_err());

        drop(started);
        clear_on_logout().expect("logout cleanup");

        assert!(assert_preview_allowed().is_err());
        assert!(
            inspect_and_clear_matching("dev", "other", "portfolio-1")
                .expect("different binding")
                .is_none()
        );
        assert!(assert_preview_allowed().is_err());
        let cleared = inspect_and_clear_matching("dev", "account-1", "portfolio-1")
            .expect("matching binding")
            .expect("matching marker");
        assert!(cleared.cleared_unknown_submission_gate);
        assert_preview_allowed().expect("matching inspection clears unknown gate");
    }

    #[test]
    fn corrupt_store_fails_with_stable_error_prefix() {
        let (_temp, _guard) = setup();
        let path = confirmation_file_path().expect("path");
        fs::write(path, "not-json").expect("write corrupt fixture");

        let err = load_pending("scsp1_corrupt", 100).expect_err("corrupt state must fail");
        assert!(
            err.to_string()
                .contains("SAVINGS_PLAN_CONFIRMATION_CORRUPT:")
        );
    }

    #[test]
    fn tampered_pending_snapshot_fails_checksum_validation() {
        let (_temp, _guard) = setup();
        store_pending(sample_confirmation("scsp1_tampered")).expect("save pending");
        let path = confirmation_file_path().expect("path");
        let mut stored: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read stored confirmation"))
                .expect("stored JSON");
        stored["state"]["value"]["snapshot"]["costs"]["entry"] =
            Value::String("changed".to_string());
        fs::write(
            &path,
            serde_json::to_vec(&stored).expect("serialize tampered store"),
        )
        .expect("overwrite test fixture");

        let err =
            load_pending("scsp1_tampered", 101).expect_err("tampered snapshot must fail closed");
        assert!(
            err.to_string()
                .contains("SAVINGS_PLAN_CONFIRMATION_CORRUPT:")
        );
    }
}
