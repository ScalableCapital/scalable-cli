use anyhow::{Result, bail};

use crate::config::{EnvConfig, TargetEnv};
use crate::dpop::DpopRuntimeOptions;
use crate::graphql::GraphqlAccessContext;
use crate::session::{Session, SessionManager};
use crate::session_refresh::refresh_loaded_session_if_needed;
use crate::transport_security::validate_env_transport_security;

pub(crate) struct LoadedActiveSession {
    pub(crate) session: Session,
    pub(crate) access_context: GraphqlAccessContext,
}

pub(crate) fn load_active_session(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    dpop_options: &DpopRuntimeOptions,
) -> Result<LoadedActiveSession> {
    validate_env_transport_security(env_cfg)?;
    let stored = session_manager.load_required_active()?;
    if stored.env != env {
        bail!(
            "Stored session belongs to {}, not {env}. Run 'sc login' to replace it.",
            stored.env
        );
    }
    let access_context = GraphqlAccessContext::with_session_mode(stored.mode);
    let session =
        refresh_loaded_session_if_needed(session_manager, env, env_cfg, stored, dpop_options)?;
    Ok(LoadedActiveSession {
        session,
        access_context,
    })
}
