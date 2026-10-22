use anyhow::{Context, Result, bail};

use crate::auth::{
    PendingSessionRefresh, REFRESH_RELOGIN_REQUIRED_PREFIX, RefreshFailure, current_epoch_seconds,
    load_authenticated_session_dpop_context, refresh_relogin_required,
    refresh_retryable_with_error, refresh_session_if_needed_with_dpop, refresh_session_with_dpop,
    session_access_token_is_expired_at, session_refresh_is_due_at,
};
use crate::config::{EnvConfig, TargetEnv};
use crate::dpop::DpopRuntimeOptions;
use crate::session::{SecretStore, Session, SessionManager, SessionRefreshLock, StoredSession};
use crate::user_error_message;

pub(crate) fn refresh_loaded_session_if_needed<S: SecretStore>(
    session_manager: &mut SessionManager<S>,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    stored_session: StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Session> {
    let now_epoch_seconds = current_epoch_seconds();
    let refresh_due = session_refresh_is_due_at(&stored_session.session, now_epoch_seconds);
    let requires_refresh_lock = stored_session.refresh_recovery_pending
        || (refresh_due && stored_session.session.refresh_token.is_some());

    if !requires_refresh_lock {
        if refresh_due
            && stored_session.session.refresh_token.is_none()
            && session_access_token_is_expired_at(&stored_session.session, now_epoch_seconds)
        {
            bail!(
                "Session access token is expired and no refresh token is available. Run 'sc login'."
            );
        }

        // The eligibility decision above is deliberately the only time-based
        // decision outside the refresh lock. A normal, usable session returns
        // immediately rather than crossing the skew window and refreshing
        // without serialization.
        let _dpop = load_authenticated_session_dpop_context(&stored_session, dpop_options)?;
        return Ok(stored_session.session);
    }

    // Keep the lock through reload, refresh, verification, and persistence so
    // a second process observes either the completed normal session or the
    // explicit recovery record, never an unverified access token.
    let _refresh_lock = SessionRefreshLock::acquire()?;
    let (stored_session, dpop) = load_compatible_active_session_after_refresh_race(
        session_manager,
        env,
        &stored_session,
        dpop_options,
    )?;
    let pending = if stored_session.refresh_recovery_pending {
        if stored_session
            .session
            .refresh_token
            .as_deref()
            .is_none_or(|token| token.trim().is_empty())
        {
            return Err(handle_refresh_failure(
                session_manager,
                &stored_session,
                refresh_relogin_required(
                    "Stored refresh recovery state has no usable refresh token",
                ),
            ));
        }

        refresh_session_with_dpop(env_cfg, &stored_session.session, &dpop).map(Some)
    } else {
        refresh_session_if_needed_with_dpop(env_cfg, &stored_session.session, &dpop)
    }
    .map_err(|error| handle_refresh_failure(session_manager, &stored_session, error))?;

    if let Some(pending) = pending {
        return persist_pending_refresh(
            session_manager,
            &stored_session,
            pending,
            env_cfg,
            dpop_options,
        );
    }

    Ok(stored_session.session)
}

pub(crate) fn execute_with_refresh_retry<T, F, S: SecretStore>(
    session_manager: &mut SessionManager<S>,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    session: &mut Session,
    dpop_options: &DpopRuntimeOptions,
    mut action: F,
) -> Result<T>
where
    F: FnMut(&str) -> Result<T>,
{
    match action(&session.access_token) {
        Ok(value) => Ok(value),
        Err(first_err)
            if is_unauthorized_graphql_error(&first_err) && session.refresh_token.is_some() =>
        {
            let authoritative = {
                let _refresh_lock = SessionRefreshLock::acquire()?;
                let (stored_session, dpop) = load_validated_active_session_with_dpop_context(
                    session_manager,
                    env,
                    dpop_options,
                )?;
                if stored_session.session.person_id != session.person_id {
                    bail!("Active session changed to a different identity. Re-run the command.");
                }
                let pending = refresh_session_with_dpop(env_cfg, &stored_session.session, &dpop)
                    .map_err(|error| {
                        handle_refresh_failure(session_manager, &stored_session, error)
                            .context("Token refresh after unauthorized response failed")
                    })?;
                persist_pending_refresh(
                    session_manager,
                    &stored_session,
                    pending,
                    env_cfg,
                    dpop_options,
                )?
            };
            *session = authoritative;

            action(&session.access_token)
                .context("GraphQL call failed after refreshing access token")
        }
        Err(first_err) => Err(first_err),
    }
}

fn is_unauthorized_graphql_error(err: &anyhow::Error) -> bool {
    let detail = err.to_string();
    if detail.contains("GraphQL HTTP error 401") {
        return true;
    }

    // Some GraphQL backends return auth failures in a 200 payload with errors[].
    if detail.contains("GraphQL returned errors")
        && (detail.contains("UNAUTHENTICATED")
            || detail.contains("\"code\":\"UNAUTHENTICATED\"")
            || detail.contains("Missing or invalid credentials"))
    {
        return true;
    }

    false
}

enum SessionSaveOutcome {
    Saved(StoredSession),
    Replaced(StoredSession),
}

#[derive(Debug)]
enum SessionSaveError {
    Storage(anyhow::Error),
    Concurrent(anyhow::Error),
}

fn persist_pending_refresh<S: SecretStore>(
    session_manager: &mut SessionManager<S>,
    stale_session: &StoredSession,
    pending: PendingSessionRefresh,
    env_cfg: &EnvConfig,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Session> {
    let recovery_session = pending.recovery_session(&stale_session.session);
    let recovery_stored = match save_rotated_refresh_token(
        session_manager,
        recovery_session,
        stale_session,
        dpop_options,
    )
    .map_err(|err| handle_refresh_failure(session_manager, stale_session, err))?
    {
        SessionSaveOutcome::Saved(stored) => stored,
        SessionSaveOutcome::Replaced(stored) if !stored.refresh_recovery_pending => {
            return Ok(stored.session);
        }
        SessionSaveOutcome::Replaced(_) => {
            return Err(refresh_retryable_with_error(
                "Token refresh could not replace an active recovery state. Retry the command",
                anyhow::anyhow!("active session changed to refresh recovery"),
            )
            .into_error());
        }
    };

    let refreshed = pending
        .verify(env_cfg, &stale_session.session)
        .map_err(|err| handle_refresh_failure(session_manager, &recovery_stored, err))?;
    let authoritative =
        save_verified_refreshed_session(session_manager, refreshed, &recovery_stored, dpop_options)
            .map_err(|err| handle_refresh_failure(session_manager, &recovery_stored, err))?;

    match authoritative {
        SessionSaveOutcome::Saved(stored) => Ok(stored.session),
        SessionSaveOutcome::Replaced(stored) if !stored.refresh_recovery_pending => {
            Ok(stored.session)
        }
        SessionSaveOutcome::Replaced(_) => Err(refresh_retryable_with_error(
            "Token refresh could not finalize because the active session remains in recovery. Retry the command",
            anyhow::anyhow!("active session changed to refresh recovery"),
        ).into_error()),
    }
}

fn save_rotated_refresh_token<S: SecretStore>(
    session_manager: &mut SessionManager<S>,
    session: Session,
    base_session: &StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> std::result::Result<SessionSaveOutcome, RefreshFailure> {
    let recovery = StoredSession {
        session,
        refresh_recovery_pending: true,
        ..base_session.clone()
    };
    match save_active_session(session_manager, base_session, recovery, dpop_options) {
        Ok(outcome) => Ok(outcome),
        Err(SessionSaveError::Storage(_)) => Err(refresh_relogin_required(
            "Token refresh succeeded but the rotated refresh token could not be persisted locally",
        )),
        Err(SessionSaveError::Concurrent(error)) => Err(RefreshFailure::Other(error)),
    }
}

fn save_verified_refreshed_session<S: SecretStore>(
    session_manager: &mut SessionManager<S>,
    session: Session,
    base_session: &StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> std::result::Result<SessionSaveOutcome, RefreshFailure> {
    let verified = StoredSession {
        session,
        refresh_recovery_pending: false,
        ..base_session.clone()
    };
    match save_active_session(session_manager, base_session, verified, dpop_options) {
        Ok(outcome) => Ok(outcome),
        Err(SessionSaveError::Storage(error)) => Err(refresh_retryable_with_error(
            "Token refresh succeeded and its replacement refresh token was saved, but the verified access token could not be persisted locally. Retry the command",
            error,
        )),
        Err(SessionSaveError::Concurrent(error)) => Err(RefreshFailure::Other(error)),
    }
}

fn save_active_session<S: SecretStore>(
    session_manager: &mut SessionManager<S>,
    base_session: &StoredSession,
    replacement: StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> std::result::Result<SessionSaveOutcome, SessionSaveError> {
    match session_manager.save_active_if_matches(base_session, &replacement) {
        Ok(true) => Ok(SessionSaveOutcome::Saved(replacement)),
        Ok(false) => load_compatible_active_session_after_refresh_race(
            session_manager,
            replacement.env,
            base_session,
            dpop_options,
        )
        .context(
            "Token refresh succeeded, but another process changed the active session before it could be persisted",
        )
        .map(|(stored, _dpop)| SessionSaveOutcome::Replaced(stored))
        .map_err(SessionSaveError::Concurrent),
        Err(err) => Err(SessionSaveError::Storage(err)),
    }
}

fn load_compatible_active_session_after_refresh_race<S: SecretStore>(
    session_manager: &SessionManager<S>,
    env: TargetEnv,
    base_session: &StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> Result<(StoredSession, crate::auth::AuthDpopContext)> {
    let (stored, dpop) =
        load_validated_active_session_with_dpop_context(session_manager, env, dpop_options)?;
    if stored.session.person_id != base_session.session.person_id
        || stored.mode != base_session.mode
    {
        bail!(
            "Active session changed to a different identity or session mode. Re-run the command."
        );
    }
    Ok((stored, dpop))
}

fn load_validated_active_session<S: SecretStore>(
    session_manager: &SessionManager<S>,
    env: TargetEnv,
) -> Result<StoredSession> {
    let stored = session_manager.load_required_active()?;
    if stored.env != env {
        bail!(
            "Stored session belongs to {}, not {env}. Run 'sc login' to replace it.",
            stored.env
        );
    }
    Ok(stored)
}

fn load_validated_active_session_with_dpop_context<S: SecretStore>(
    session_manager: &SessionManager<S>,
    env: TargetEnv,
    dpop_options: &DpopRuntimeOptions,
) -> Result<(StoredSession, crate::auth::AuthDpopContext)> {
    let stored = load_validated_active_session(session_manager, env)?;
    let dpop = load_authenticated_session_dpop_context(&stored, dpop_options)?;
    Ok((stored, dpop))
}

fn handle_refresh_failure<S: SecretStore>(
    session_manager: &mut SessionManager<S>,
    stale_session: &StoredSession,
    error: RefreshFailure,
) -> anyhow::Error {
    match error {
        RefreshFailure::IssuerMaterialUnavailable(error)
            if stale_session.refresh_recovery_pending =>
        {
            refresh_retryable_with_error(
                "Refresh recovery is waiting for issuer metadata or signing keys. Retry the command",
                error,
            ).into_error()
        }
        RefreshFailure::ReloginRequired(error) => {
            match session_manager.delete_active_if_matches(stale_session) {
                Ok(true) | Ok(false) => error,
                Err(delete_err) => anyhow::anyhow!(
                    "{REFRESH_RELOGIN_REQUIRED_PREFIX} {} Failed clearing the stale active session locally: {delete_err}",
                    user_error_message(&error)
                ),
            }
        }
        RefreshFailure::IssuerMaterialUnavailable(error)
        | RefreshFailure::Retryable(error)
        | RefreshFailure::Other(error) => error,
    }
}

#[cfg(test)]
#[path = "session_refresh_tests.rs"]
mod tests;
