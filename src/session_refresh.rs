use anyhow::{Context, Result, bail};

use crate::auth::{
    REFRESH_RELOGIN_REQUIRED_PREFIX, load_authenticated_session_dpop_context,
    refresh_session_if_needed_with_dpop, refresh_session_with_dpop,
};
use crate::config::{EnvConfig, TargetEnv};
use crate::dpop::DpopRuntimeOptions;
use crate::session::{Session, SessionManager, SessionMode, StoredSession};
use crate::user_error_message;

pub(crate) fn refresh_loaded_session_if_needed(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    stored_session: StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Session> {
    let dpop = load_authenticated_session_dpop_context(&stored_session, dpop_options)?;
    if let Some(refreshed) = refresh_session_if_needed_with_dpop(
        env_cfg,
        &stored_session.session,
        &dpop,
    )
    .map_err(|err| {
        clear_active_session_on_refresh_relogin_failure(session_manager, Some(&stored_session), err)
    })? {
        let authoritative = save_active_session(
            session_manager,
            env,
            &refreshed,
            &stored_session,
            stored_session.mode,
            dpop_options,
            dpop.jwk_thumbprint(),
        )
        .map_err(|err| {
            clear_active_session_on_refresh_relogin_failure(
                session_manager,
                Some(&stored_session),
                err,
            )
        })?;
        return Ok(authoritative.session);
    }

    Ok(stored_session.session)
}

pub(crate) fn execute_with_refresh_retry<T, F>(
    session_manager: &mut SessionManager,
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
            let (stored_session, dpop) = load_validated_active_session_with_dpop_context(
                session_manager,
                env,
                dpop_options,
            )?;
            let refreshed = refresh_session_with_dpop(env_cfg, &stored_session.session, &dpop)
                .context("Token refresh after unauthorized response failed")
                .map_err(|err| {
                    clear_active_session_on_refresh_relogin_failure(
                        session_manager,
                        Some(&stored_session),
                        err,
                    )
                })?;
            let authoritative = save_active_session(
                session_manager,
                env,
                &refreshed,
                &stored_session,
                stored_session.mode,
                dpop_options,
                dpop.jwk_thumbprint(),
            )
            .map_err(|err| {
                clear_active_session_on_refresh_relogin_failure(
                    session_manager,
                    Some(&stored_session),
                    err,
                )
            })?;
            *session = authoritative.session;

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

fn save_active_session(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    session: &Session,
    base_session: &StoredSession,
    session_mode: Option<SessionMode>,
    dpop_options: &DpopRuntimeOptions,
    dpop_jwk_thumbprint: &str,
) -> Result<StoredSession> {
    let replacement = StoredSession {
        env,
        session: session.clone(),
        dpop_jwk_thumbprint: Some(dpop_jwk_thumbprint.to_string()),
        mode: session_mode,
    };
    match session_manager.save_active_if_matches(base_session, &replacement) {
        Ok(true) => Ok(replacement),
        Ok(false) => load_compatible_active_session_after_refresh_race(
            session_manager,
            env,
            base_session,
            dpop_options,
        )
        .context(
            "Token refresh succeeded, but another process changed the active session before it could be persisted",
        ),
        Err(_err) => Err(anyhow::anyhow!(
            "{REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh succeeded but the rotated session could not be persisted locally. Run 'sc login'."
        )),
    }
}

fn load_compatible_active_session_after_refresh_race(
    session_manager: &SessionManager,
    env: TargetEnv,
    base_session: &StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> Result<StoredSession> {
    let (stored, _dpop) =
        load_validated_active_session_with_dpop_context(session_manager, env, dpop_options)?;
    if stored.session.person_id != base_session.session.person_id
        || stored.mode != base_session.mode
    {
        bail!(
            "Active session changed to a different identity or session mode. Re-run the command."
        );
    }
    Ok(stored)
}

fn load_validated_active_session(
    session_manager: &SessionManager,
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

fn load_validated_active_session_with_dpop_context(
    session_manager: &SessionManager,
    env: TargetEnv,
    dpop_options: &DpopRuntimeOptions,
) -> Result<(StoredSession, crate::auth::AuthDpopContext)> {
    let stored = load_validated_active_session(session_manager, env)?;
    let dpop = load_authenticated_session_dpop_context(&stored, dpop_options)?;
    Ok((stored, dpop))
}

fn clear_active_session_on_refresh_relogin_failure(
    session_manager: &mut SessionManager,
    stale_session: Option<&StoredSession>,
    err: anyhow::Error,
) -> anyhow::Error {
    if !error_chain_contains_refresh_relogin_prefix(&err) {
        return err;
    }

    let deleted = match stale_session {
        Some(stale_session) => session_manager.delete_active_if_matches(stale_session),
        None => session_manager.delete_active_locked().map(|()| true),
    };

    match deleted {
        Ok(true) | Ok(false) => err,
        Err(delete_err) => anyhow::anyhow!(
            "{REFRESH_RELOGIN_REQUIRED_PREFIX} {} Failed clearing the stale active session locally: {delete_err}",
            user_error_message(&err)
        ),
    }
}

fn error_chain_contains_refresh_relogin_prefix(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.to_string().contains(REFRESH_RELOGIN_REQUIRED_PREFIX))
}

#[cfg(test)]
#[path = "session_refresh_tests.rs"]
mod tests;
