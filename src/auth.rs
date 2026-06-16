use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::json;

use crate::config::{EnvConfig, TargetEnv};
use crate::dpop::{DPOP_SESSION_KEY_RELOGIN_MESSAGE, DpopKeyMaterial, DpopRuntimeOptions};
use crate::graphql::{
    GraphqlAccessContext, execute_graphql, fetch_login_2fa_state, start_2fa_on_login,
    validate_2fa_on_login,
};
use crate::session::{
    LoginSource, SecretWriteBackend, Session, SessionManager, SessionMode,
    StorageBackendDiagnostics, StoredSession,
};
use crate::token_verifier::{verify_access_token_allow_expired, verify_access_token_strict};
use crate::transport_security::{
    RUNTIME_HTTP_TIMEOUT, build_blocking_client_https_only_with_timeout,
    validate_env_transport_security, validate_https_url,
};

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: i64,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DeviceErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug)]
enum DevicePollState {
    Authorized(TokenResponse),
    Pending,
    SlowDown,
}

const FILE_SECRET_STORAGE_WARNING_LABEL: &str = "Warning";
const FILE_SECRET_STORAGE_WARNING_BODY: &str = "session secrets are stored in local files. Prefer keyring-backed session storage on this device.";
const ANSI_BOLD_PREFIX: &str = "\x1b[1m";
const ANSI_RESET: &str = "\x1b[0m";

pub(crate) struct AuthDpopContext {
    key_material: DpopKeyMaterial,
    jwk_thumbprint: String,
}

impl AuthDpopContext {
    fn from_key_material(key_material: DpopKeyMaterial) -> Result<Self> {
        let jwk_thumbprint = key_material
            .jwk_thumbprint()
            .context("Failed computing DPoP key thumbprint")?;
        Ok(Self {
            key_material,
            jwk_thumbprint,
        })
    }

    fn from_runtime_options_for_login(options: &DpopRuntimeOptions) -> Result<Self> {
        let key_material = DpopKeyMaterial::load_or_create_for_options(options)
            .context("Failed to load or create DPoP key material")?;
        Self::from_key_material(key_material)
    }

    fn from_runtime_options_for_authenticated_session(
        options: &DpopRuntimeOptions,
    ) -> Result<Self> {
        let key_material = DpopKeyMaterial::load_existing_for_options(options)
            .context(DPOP_SESSION_KEY_RELOGIN_MESSAGE)?;
        Self::from_key_material(key_material).context(DPOP_SESSION_KEY_RELOGIN_MESSAGE)
    }

    pub(crate) fn jwk_thumbprint(&self) -> &str {
        &self.jwk_thumbprint
    }

    #[cfg(test)]
    fn with_key_material_for_tests(key_material: DpopKeyMaterial) -> Self {
        Self::from_key_material(key_material).expect("test DPoP key should have thumbprint")
    }
}

pub(crate) fn load_authenticated_session_dpop_context(
    stored_session: &StoredSession,
    options: &DpopRuntimeOptions,
) -> Result<AuthDpopContext> {
    let expected = stored_session
        .dpop_jwk_thumbprint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context(DPOP_SESSION_KEY_RELOGIN_MESSAGE)?;
    let dpop = AuthDpopContext::from_runtime_options_for_authenticated_session(options)?;
    if dpop.jwk_thumbprint != expected {
        bail!(DPOP_SESSION_KEY_RELOGIN_MESSAGE);
    }
    Ok(dpop)
}

#[cfg(test)]
pub(crate) fn validate_stored_session_dpop_binding(
    stored_session: &StoredSession,
    options: &DpopRuntimeOptions,
) -> Result<String> {
    Ok(
        load_authenticated_session_dpop_context(stored_session, options)?
            .jwk_thumbprint
            .clone(),
    )
}

const REFRESH_SKEW_SECONDS: i64 = 60;
const AUTH_SCOPE: &str = "offline_access openid email";
const TRUSTED_DEVICE_2FA_FAILURE_MESSAGE: &str =
    "Login failed: 2FA verification on the trusted device failed.";
const SECOND_FACTOR_WAIT_MESSAGE: &str =
    "Waiting for second factor approval on your linked device...";
const BROWSER_CONFIRMATION_WAIT_MESSAGE: &str = "Waiting for browser confirmation...";
const SPINNER_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
const SPINNER_FRAME_INTERVAL: Duration = Duration::from_millis(125);
pub(crate) const REFRESH_RELOGIN_REQUIRED_PREFIX: &str = "REFRESH_RELOGIN_REQUIRED:";
const REVOKE_AUTH_ACCESS_TOKEN_MUTATION: &str = r#"
mutation revokeAuthAccessToken($input: AuthAccessTokenInput!) {
  revokeAuthAccessToken(input: $input)
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
enum LoginProgressStage {
    BrowserConfirmation,
    BrowserConfirmationSlowDown { interval_secs: u64 },
    SecondFactorApproval,
}

impl LoginProgressStage {
    fn message(&self) -> String {
        match self {
            Self::BrowserConfirmation => BROWSER_CONFIRMATION_WAIT_MESSAGE.to_string(),
            Self::BrowserConfirmationSlowDown { interval_secs } => format!(
                "{BROWSER_CONFIRMATION_WAIT_MESSAGE} (Auth server asked to slow down; polling every {interval_secs}s)"
            ),
            Self::SecondFactorApproval => SECOND_FACTOR_WAIT_MESSAGE.to_string(),
        }
    }
}

trait LoginProgressSink {
    fn note_browser_confirmation_pending(&mut self, pending_polls: u64) -> Result<()>;
    fn note_browser_confirmation_slow_down(&mut self, interval_secs: u64) -> Result<()>;
    fn wait_for_browser_confirmation(&mut self, duration: Duration) -> Result<()>;
    fn note_second_factor_wait(&mut self) -> Result<()>;
    fn wait_for_second_factor(&mut self, duration: Duration) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
}

struct TerminalLoginProgress<W: Write> {
    writer: Option<W>,
    interactive: bool,
    spinner_index: usize,
    active_stage: Option<LoginProgressStage>,
    last_render_width: usize,
    finished: bool,
}

impl<W: Write> TerminalLoginProgress<W> {
    fn new(writer: W, interactive: bool) -> Self {
        Self {
            writer: Some(writer),
            interactive,
            spinner_index: 0,
            active_stage: None,
            last_render_width: 0,
            finished: false,
        }
    }

    #[cfg(test)]
    fn into_inner(mut self) -> W {
        self.finished = true;
        self.writer
            .take()
            .expect("terminal login progress writer should exist")
    }

    fn writer_mut(&mut self) -> Result<&mut W> {
        self.writer
            .as_mut()
            .context("Login progress writer was unexpectedly unavailable")
    }

    fn set_stage(&mut self, stage: LoginProgressStage) -> Result<()> {
        self.active_stage = Some(stage);
        if self.interactive {
            self.render_active_stage()?;
        }
        Ok(())
    }

    fn render_active_stage(&mut self) -> Result<()> {
        let Some(stage) = self.active_stage.as_ref() else {
            return Ok(());
        };
        let line = format!("{} {}", SPINNER_FRAMES[self.spinner_index], stage.message());
        let padding = self.last_render_width.saturating_sub(line.len());
        let writer = self.writer_mut()?;
        write!(writer, "\r{line}")?;
        if padding > 0 {
            write!(writer, "{}", " ".repeat(padding))?;
        }
        writer.flush().context("Failed to flush login progress")?;
        self.last_render_width = line.len();
        self.spinner_index = (self.spinner_index + 1) % SPINNER_FRAMES.len();
        Ok(())
    }

    fn animate_for(&mut self, duration: Duration) -> Result<()> {
        if !self.interactive || self.active_stage.is_none() || duration.is_zero() {
            return Ok(());
        }

        let start = Instant::now();
        while let Some(remaining) = duration.checked_sub(start.elapsed()) {
            self.render_active_stage()?;
            thread::sleep(remaining.min(SPINNER_FRAME_INTERVAL));
        }

        Ok(())
    }

    fn clear_active_line(&mut self) -> Result<()> {
        if !self.interactive || self.last_render_width == 0 {
            return Ok(());
        }

        let blank = " ".repeat(self.last_render_width);
        let writer = self.writer_mut()?;
        write!(writer, "\r{blank}\r")?;
        writer
            .flush()
            .context("Failed to flush cleared login progress")?;
        self.last_render_width = 0;
        Ok(())
    }
}

impl<W: Write> LoginProgressSink for TerminalLoginProgress<W> {
    fn note_browser_confirmation_pending(&mut self, pending_polls: u64) -> Result<()> {
        if self.interactive {
            self.set_stage(LoginProgressStage::BrowserConfirmation)?;
            return Ok(());
        }

        if should_print_device_wait_message(pending_polls) {
            let writer = self.writer_mut()?;
            writeln!(writer, "{BROWSER_CONFIRMATION_WAIT_MESSAGE}")?;
            writer.flush().context("Failed to flush login progress")?;
        }
        Ok(())
    }

    fn note_browser_confirmation_slow_down(&mut self, interval_secs: u64) -> Result<()> {
        let stage = LoginProgressStage::BrowserConfirmationSlowDown { interval_secs };
        if self.interactive {
            self.set_stage(stage)?;
            return Ok(());
        }

        let writer = self.writer_mut()?;
        writeln!(writer, "{}", stage.message())?;
        writer.flush().context("Failed to flush login progress")?;
        Ok(())
    }

    fn wait_for_browser_confirmation(&mut self, duration: Duration) -> Result<()> {
        if self.interactive {
            if self.active_stage.is_none() {
                self.set_stage(LoginProgressStage::BrowserConfirmation)?;
            }
            self.animate_for(duration)?;
        } else {
            thread::sleep(duration);
        }
        Ok(())
    }

    fn note_second_factor_wait(&mut self) -> Result<()> {
        let stage = LoginProgressStage::SecondFactorApproval;
        if self.interactive {
            self.set_stage(stage)?;
            return Ok(());
        }

        if self.active_stage.as_ref() == Some(&stage) {
            return Ok(());
        }

        self.active_stage = Some(stage.clone());
        let writer = self.writer_mut()?;
        writeln!(writer, "{}", stage.message())?;
        writer.flush().context("Failed to flush login progress")?;
        Ok(())
    }

    fn wait_for_second_factor(&mut self, duration: Duration) -> Result<()> {
        if self.interactive {
            self.animate_for(duration)?;
        } else {
            thread::sleep(duration);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.clear_active_line()?;
        self.finished = true;
        Ok(())
    }
}

impl<W: Write> Drop for TerminalLoginProgress<W> {
    fn drop(&mut self) {
        if self.finished || !self.interactive {
            return;
        }
        if self.clear_active_line().is_ok()
            && let Some(writer) = self.writer.as_mut()
        {
            let _ = writeln!(writer);
            let _ = writer.flush();
        }
    }
}

fn validated_auth_issuer(env_cfg: &EnvConfig) -> Result<String> {
    Ok(validate_https_url(&env_cfg.auth.issuer, "auth.issuer")?
        .as_str()
        .trim_end_matches('/')
        .to_string())
}

fn should_print_local_file_storage_warning(diagnostics: &StorageBackendDiagnostics) -> bool {
    diagnostics.effective_backend == "file"
}

fn emphasize_terminal(text: &str, style_enabled: bool) -> String {
    if style_enabled {
        format!("{ANSI_BOLD_PREFIX}{text}{ANSI_RESET}")
    } else {
        text.to_string()
    }
}

fn format_local_file_storage_warning(style_enabled: bool) -> String {
    format!(
        "{}: {}",
        emphasize_terminal(FILE_SECRET_STORAGE_WARNING_LABEL, style_enabled),
        FILE_SECRET_STORAGE_WARNING_BODY
    )
}

fn format_device_login_prompt(
    device: &DeviceCodeResponse,
    show_local_file_storage_warning: bool,
    style_enabled: bool,
) -> String {
    let verification_url = device
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&device.verification_uri);
    let mut blocks = Vec::with_capacity(3);

    if show_local_file_storage_warning {
        blocks.push(format_local_file_storage_warning(style_enabled));
    }

    blocks.push(format!("Open this URL:\n{verification_url}"));
    blocks.push(format!(
        "Verify the code {} in your browser.",
        emphasize_terminal(&device.user_code, style_enabled)
    ));

    format!("{}\n\n", blocks.join("\n\n"))
}

fn print_device_login_prompt(
    device: &DeviceCodeResponse,
    show_local_file_storage_warning: bool,
) -> Result<()> {
    let style_enabled = io::stdout().is_terminal();
    let prompt = format_device_login_prompt(device, show_local_file_storage_warning, style_enabled);
    let mut stdout = io::stdout();
    stdout
        .write_all(prompt.as_bytes())
        .context("Failed to write login prompt")?;
    stdout.flush().context("Failed to flush login prompt")?;
    Ok(())
}

fn should_use_interactive_login_status(stdout_is_terminal: bool) -> bool {
    stdout_is_terminal
        && std::env::var("TERM")
            .map(|term| !term.eq_ignore_ascii_case("dumb"))
            .unwrap_or(true)
}

pub fn login_with_device_code(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    session_mode: Option<SessionMode>,
    dpop_options: &DpopRuntimeOptions,
) -> Result<SecretWriteBackend> {
    validate_env_transport_security(env_cfg)?;
    let client = build_blocking_client_https_only_with_timeout(RUNTIME_HTTP_TIMEOUT)?;
    let dpop = AuthDpopContext::from_runtime_options_for_login(dpop_options)?;
    let device = request_device_code(&client, env_cfg, &dpop)?;
    print_device_login_prompt(
        &device,
        should_print_local_file_storage_warning(&session_manager.storage_backend_diagnostics()),
    )?;
    let mut progress = TerminalLoginProgress::new(
        io::stdout(),
        should_use_interactive_login_status(io::stdout().is_terminal()),
    );

    let mut interval_secs = device.interval.unwrap_or(5).max(1);
    let expires_at = Instant::now() + Duration::from_secs(device.expires_in as u64);
    let mut pending_polls = 0_u64;

    loop {
        if Instant::now() >= expires_at {
            bail!("Device code login expired before completion");
        }

        progress.wait_for_browser_confirmation(Duration::from_secs(interval_secs))?;

        match poll_device_token_once(&client, env_cfg, &device.device_code, &dpop)? {
            DevicePollState::Authorized(token) => {
                let verified = verify_access_token_strict(&token.access_token, env_cfg)
                    .context("Access token verification failed after device login")?;
                handle_post_login_mfa_with_sanitized_errors(
                    &mut progress,
                    env_cfg,
                    &token.access_token,
                    &verified.person_id,
                    dpop_options,
                )?;
                progress.finish()?;

                let session = Session {
                    access_token: token.access_token,
                    refresh_token: token.refresh_token,
                    id_token: token.id_token,
                    expires_at: token.expires_in.map(|s| current_epoch_seconds() + s),
                    person_id: verified.person_id,
                    source: LoginSource::DeviceCode,
                };
                return session_manager.save_active_with_backend(&StoredSession {
                    env,
                    session,
                    dpop_jwk_thumbprint: Some(dpop.jwk_thumbprint.clone()),
                    mode: session_mode,
                });
            }
            DevicePollState::Pending => {
                pending_polls += 1;
                progress.note_browser_confirmation_pending(pending_polls)?;
                continue;
            }
            DevicePollState::SlowDown => {
                interval_secs += 2;
                progress.note_browser_confirmation_slow_down(interval_secs)?;
                continue;
            }
        }
    }
}

fn should_print_device_wait_message(pending_polls: u64) -> bool {
    pending_polls == 1 || pending_polls.is_multiple_of(3)
}

#[cfg(test)]
pub(crate) fn refresh_session_if_needed(
    env_cfg: &EnvConfig,
    session: &Session,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Option<Session>> {
    validate_env_transport_security(env_cfg)?;
    let dpop = AuthDpopContext::from_runtime_options_for_authenticated_session(dpop_options)?;
    refresh_session_if_needed_with_dpop(env_cfg, session, &dpop)
}

pub(crate) fn refresh_session_if_needed_with_dpop(
    env_cfg: &EnvConfig,
    session: &Session,
    dpop: &AuthDpopContext,
) -> Result<Option<Session>> {
    validate_env_transport_security(env_cfg)?;
    refresh_session_if_needed_at(env_cfg, session, current_epoch_seconds(), dpop)
}

pub(crate) fn refresh_session_with_dpop(
    env_cfg: &EnvConfig,
    session: &Session,
    dpop: &AuthDpopContext,
) -> Result<Session> {
    validate_env_transport_security(env_cfg)?;
    refresh_session_at(env_cfg, session, current_epoch_seconds(), dpop)
}

pub fn revoke_tokens_on_logout_best_effort(
    env_cfg: &EnvConfig,
    session: &Session,
    session_mode: Option<SessionMode>,
    dpop_options: &DpopRuntimeOptions,
) {
    if let Some(refresh_token) = session.refresh_token.as_deref() {
        let _ = revoke_refresh_token_on_logout(env_cfg, refresh_token, dpop_options);
    }
    let _ =
        revoke_access_token_on_logout(env_cfg, &session.access_token, session_mode, dpop_options);
}

fn refresh_session_if_needed_at(
    env_cfg: &EnvConfig,
    session: &Session,
    now_epoch_seconds: i64,
    dpop: &AuthDpopContext,
) -> Result<Option<Session>> {
    if !is_expiring_soon(session.expires_at, now_epoch_seconds) {
        return Ok(None);
    }

    if session.refresh_token.is_none() {
        if session
            .expires_at
            .is_some_and(|exp| exp <= now_epoch_seconds)
        {
            bail!(
                "Session access token is expired and no refresh token is available. Run 'sc login'."
            );
        }
        return Ok(None);
    }

    let refreshed = refresh_session_at(env_cfg, session, now_epoch_seconds, dpop)?;
    Ok(Some(refreshed))
}

fn refresh_session_at(
    env_cfg: &EnvConfig,
    session: &Session,
    now_epoch_seconds: i64,
    dpop: &AuthDpopContext,
) -> Result<Session> {
    let refresh_token = session
        .refresh_token
        .as_deref()
        .context("No refresh token is available for this session")?;
    let session_id = session_id_claim_from_access_token(env_cfg, &session.access_token)
        .context("Existing access token failed verification during refresh")?;

    let client = build_blocking_client_https_only_with_timeout(RUNTIME_HTTP_TIMEOUT)?;
    let token = refresh_access_token(&client, env_cfg, refresh_token, session_id.as_deref(), dpop)?;

    let rotated_refresh_token = token
        .refresh_token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            refresh_relogin_required(
                "Token refresh succeeded but no usable replacement refresh token was returned",
            )
        })?;
    let verified = verify_access_token_strict(&token.access_token, env_cfg).map_err(|_| {
        refresh_relogin_required(
            "Token refresh succeeded but the refreshed access token failed verification",
        )
    })?;

    Ok(Session {
        access_token: token.access_token,
        refresh_token: Some(rotated_refresh_token),
        id_token: token.id_token.or_else(|| session.id_token.clone()),
        expires_at: token
            .expires_in
            .map(|s| now_epoch_seconds + s)
            .or(verified.expires_at),
        person_id: verified.person_id,
        source: session.source.clone(),
    })
}

fn is_expiring_soon(expires_at: Option<i64>, now_epoch_seconds: i64) -> bool {
    expires_at
        .map(|exp| exp <= now_epoch_seconds + REFRESH_SKEW_SECONDS)
        .unwrap_or(false)
}

fn request_device_code(
    client: &Client,
    env_cfg: &EnvConfig,
    dpop: &AuthDpopContext,
) -> Result<DeviceCodeResponse> {
    let issuer = validated_auth_issuer(env_cfg)?;

    let mut start_form = HashMap::new();
    start_form.insert("client_id", env_cfg.auth.client_id.as_str());
    start_form.insert("audience", env_cfg.auth.audience.as_str());
    start_form.insert("scope", AUTH_SCOPE);

    send_oauth_form(
        client,
        &format!("{issuer}/oauth/device/code"),
        &start_form,
        "Device code request",
        dpop,
    )?
    .json()
    .context("Failed parsing device code response")
}

fn poll_device_token_once(
    client: &Client,
    env_cfg: &EnvConfig,
    device_code: &str,
    dpop: &AuthDpopContext,
) -> Result<DevicePollState> {
    let issuer = validated_auth_issuer(env_cfg)?;
    let mut poll_form = HashMap::new();
    poll_form.insert("grant_type", "urn:ietf:params:oauth:grant-type:device_code");
    poll_form.insert("device_code", device_code);
    poll_form.insert("client_id", env_cfg.auth.client_id.as_str());

    let url = format!("{issuer}/oauth/token");
    let mut nonce = None::<String>;
    for attempt in 0..2 {
        let mut request = client.post(&url).form(&poll_form);
        let proof = dpop
            .key_material
            .proof_for_request("POST", &url, nonce.as_deref(), None)
            .context("Failed generating DPoP proof for device token polling")?;
        request = request.header("DPoP", proof);

        let response = request.send().context("Failed polling token endpoint")?;

        if response.status().is_success() {
            let token: TokenResponse = response
                .json()
                .context("Failed parsing device flow token response")?;
            return Ok(DevicePollState::Authorized(token));
        }

        let status = response.status();
        let retry_nonce = dpop_nonce_from_headers(response.headers());
        let body = response.text().unwrap_or_default();
        if attempt == 0 && should_retry_with_dpop_nonce(status, retry_nonce.as_deref(), &body) {
            nonce = retry_nonce;
            continue;
        }

        let err: DeviceErrorResponse = serde_json::from_str(&body).unwrap_or(DeviceErrorResponse {
            error: "unknown_error".to_string(),
            error_description: Some(body.clone()),
        });

        return match err.error.as_str() {
            "authorization_pending" => Ok(DevicePollState::Pending),
            "slow_down" => Ok(DevicePollState::SlowDown),
            "access_denied" => bail!("Device login denied by user"),
            "expired_token" => bail!("Device login code expired"),
            _ => {
                let message = format_oauth_error("Device Code token polling", status, &body);
                bail!(with_dpop_troubleshooting_hint(message, status, &body))
            }
        };
    }

    unreachable!("device token polling retry loop should always return");
}

fn refresh_access_token(
    client: &Client,
    env_cfg: &EnvConfig,
    refresh_token: &str,
    session_id: Option<&str>,
    dpop: &AuthDpopContext,
) -> Result<TokenResponse> {
    let issuer = validated_auth_issuer(env_cfg)?;

    let mut form = HashMap::new();
    form.insert("grant_type", "refresh_token");
    form.insert("client_id", env_cfg.auth.client_id.as_str());
    form.insert("refresh_token", refresh_token);
    if let Some(value) = session_id.map(str::trim).filter(|value| !value.is_empty()) {
        form.insert("session_id", value);
    }

    send_oauth_form(
        client,
        &format!("{issuer}/oauth/token"),
        &form,
        "Token refresh",
        dpop,
    )?
    .json()
    .context("Failed parsing refresh token response")
}

fn revoke_refresh_token_on_logout(
    env_cfg: &EnvConfig,
    refresh_token: &str,
    dpop_options: &DpopRuntimeOptions,
) -> Result<()> {
    let issuer = validated_auth_issuer(env_cfg)?;
    let client = build_blocking_client_https_only_with_timeout(RUNTIME_HTTP_TIMEOUT)?;
    let mut form = HashMap::new();
    form.insert("token", refresh_token);
    form.insert("client_id", env_cfg.auth.client_id.as_str());

    let dpop = AuthDpopContext::from_runtime_options_for_authenticated_session(dpop_options)?;

    send_oauth_form(
        &client,
        &format!("{issuer}/oauth/revoke"),
        &form,
        "Logout refresh token revoke",
        &dpop,
    )?;
    Ok(())
}

fn revoke_access_token_on_logout(
    env_cfg: &EnvConfig,
    access_token: &str,
    session_mode: Option<SessionMode>,
    dpop_options: &DpopRuntimeOptions,
) -> Result<()> {
    execute_graphql(
        &env_cfg.graphql_url,
        access_token,
        REVOKE_AUTH_ACCESS_TOKEN_MUTATION,
        &json!({
            "input": {
                "accessToken": access_token,
            }
        }),
        Some("revokeAuthAccessToken"),
        GraphqlAccessContext::with_session_mode(session_mode),
        dpop_options,
    )?;
    Ok(())
}

fn session_id_claim_from_access_token(
    env_cfg: &EnvConfig,
    access_token: &str,
) -> Result<Option<String>> {
    let verified = verify_access_token_allow_expired(access_token, env_cfg)?;
    Ok(verified.session_id)
}

fn send_oauth_form(
    client: &Client,
    url: &str,
    form: &HashMap<&str, &str>,
    action: &str,
    dpop: &AuthDpopContext,
) -> Result<reqwest::blocking::Response> {
    let validated_url = validate_https_url(url, "auth endpoint URL")
        .with_context(|| format!("Invalid URL for {action}: {url}"))?
        .to_string();
    let mut nonce = None::<String>;
    for attempt in 0..2 {
        let mut request = client.post(&validated_url).form(form);
        let proof = dpop
            .key_material
            .proof_for_request("POST", &validated_url, nonce.as_deref(), None)
            .with_context(|| format!("Failed generating DPoP proof for {action}"))?;
        request = request.header("DPoP", proof);

        let response = request
            .send()
            .with_context(|| format!("Failed performing {action}"))?;

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let retry_nonce = dpop_nonce_from_headers(response.headers());
        let body = response.text().unwrap_or_default();
        if attempt == 0 && should_retry_with_dpop_nonce(status, retry_nonce.as_deref(), &body) {
            nonce = retry_nonce;
            continue;
        }

        let message = format_oauth_error(action, status, &body);
        bail!(with_dpop_troubleshooting_hint(message, status, &body));
    }

    unreachable!("OAuth form retry loop should always return");
}

fn dpop_nonce_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("DPoP-Nonce")
        .or_else(|| headers.get("dpop-nonce"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn should_retry_with_dpop_nonce(
    status: StatusCode,
    nonce: Option<&str>,
    response_body: &str,
) -> bool {
    let Some(nonce_value) = nonce else {
        return false;
    };
    if nonce_value.is_empty() {
        return false;
    }

    let lower_body = response_body.to_lowercase();
    status == StatusCode::UNAUTHORIZED
        || lower_body.contains("use_dpop_nonce")
        || lower_body.contains("invalid_dpop_proof")
}

fn format_oauth_error(action: &str, status: StatusCode, body: &str) -> String {
    let parsed = serde_json::from_str::<DeviceErrorResponse>(body).ok();
    let code = parsed
        .as_ref()
        .map(|p| p.error.as_str())
        .unwrap_or("unknown_error");
    let desc = parsed
        .as_ref()
        .and_then(|p| p.error_description.as_deref())
        .unwrap_or(body)
        .trim();

    if action == "Token refresh" && code == "invalid_grant" {
        return format!(
            "{REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh requires a new login (OAuth {} - {code}). Run 'sc login'.",
            status.as_u16()
        );
    }

    let mut message = format!("{action} failed (OAuth {} - {code})", status.as_u16());

    let lower_desc = desc.to_lowercase();
    match code {
        "unauthorized_client" | "unsupported_grant_type" => {
            message.push_str(
                ". The required grant type is likely not enabled for this application (Device Code or Refresh Token).",
            );
        }
        "invalid_request" if lower_desc.contains("audience") => {
            message.push_str(
                ". Check OAuth API audience configuration and ensure it matches the CLI environment config.",
            );
        }
        "invalid_request" if lower_desc.contains("scope") => {
            message.push_str(". Check requested scopes in OAuth and application API permissions.");
        }
        _ => {}
    }

    message
}

fn with_dpop_troubleshooting_hint(message: String, status: StatusCode, body: &str) -> String {
    let lower_body = body.to_lowercase();
    if status == StatusCode::UNAUTHORIZED
        && (lower_body.contains("use_dpop_nonce")
            || lower_body.contains("invalid_dpop_proof")
            || lower_body.contains("\"dpop"))
    {
        return format!(
            "{message}. DPoP validation failed; verify your system clock is in sync and retry."
        );
    }

    message
}

fn handle_post_login_mfa_with_sanitized_errors(
    progress: &mut impl LoginProgressSink,
    env_cfg: &EnvConfig,
    token: &str,
    user_id: &str,
    dpop_options: &DpopRuntimeOptions,
) -> Result<()> {
    match handle_post_login_mfa(progress, env_cfg, token, user_id, dpop_options) {
        Ok(()) => Ok(()),
        Err(err) => Err(sanitize_post_login_mfa_error(err)),
    }
}

fn handle_post_login_mfa(
    progress: &mut impl LoginProgressSink,
    env_cfg: &EnvConfig,
    token: &str,
    user_id: &str,
    dpop_options: &DpopRuntimeOptions,
) -> Result<()> {
    let endpoint = &env_cfg.graphql_url;

    let state = match fetch_login_2fa_state(endpoint, token, user_id, dpop_options) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    if state.enabled != Some(true) {
        return Ok(());
    }

    if state.has_approved_session == Some(true) {
        return Ok(());
    }

    let mfa_session = match start_2fa_on_login(endpoint, token, user_id, dpop_options)? {
        Some(id) => id,
        None => bail!("2FA on login is enabled but mfaSessionId was not returned"),
    };

    progress.note_second_factor_wait()?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if Instant::now() >= deadline {
            bail!("2FA login challenge timed out")
        }

        let status = validate_2fa_on_login(endpoint, token, user_id, &mfa_session, dpop_options)?;
        match status.as_deref() {
            Some("SUCCESS") => return Ok(()),
            Some("PENDING") => {
                progress.wait_for_second_factor(Duration::from_secs(2))?;
            }
            Some("DENY") => bail!("2FA login challenge denied"),
            Some("TIMEOUT_RETRY") => bail!("2FA login timed out; please retry"),
            Some(other) => bail!("Unexpected 2FA status: {other}"),
            None => bail!("Missing 2FA status in response"),
        }
    }
}

fn sanitize_post_login_mfa_error(_: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!(TRUSTED_DEVICE_2FA_FAILURE_MESSAGE)
}

fn current_epoch_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn refresh_relogin_required(detail: &str) -> anyhow::Error {
    anyhow::anyhow!("{REFRESH_RELOGIN_REQUIRED_PREFIX} {detail}. Run 'sc login'.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::dpop::DpopKeyMaterial;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use mockito::Matcher::PartialJson;
    use mockito::{Matcher, Mock, Server};
    use serde_json::{Value, json};

    const TEST_KID: &str = "test-kid";
    const TEST_RSA_N: &str = "0hw4qSyPOM59HeFtuc2a4X-rWKPeAXmvtNHV86mYXh8TWEGc68MPq3V7NYUN-ZHulHPl2FQAMVFt-jWOQy4nJEPp1ak-P0_3Vn3Tc4lwDJXuIBC8RHPG8XMUTj7QsZPM-pf_TO_JyLrlItRZ_xADF6b_fwOhOnK5_UXLm0ZqfCYfMGFXB9Oag_Xv5yHqrHlpTkn-zYSmsC9Nk_FOsN1Apsa1Kfwj9SvRplAm86Azc3obHjHkdiOKximtWJWGi-a5eq3fYRk_csIIxO0DHGYk9rUlUQUiwr_MqJqSVc_xvi6w8j-0_Ca34BsonihOdGrNuA1yzll51pxywREMc07JkQ";
    const TEST_RSA_E: &str = "AQAB";
    const TEST_PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDSHDipLI84zn0d
4W25zZrhf6tYo94Bea+00dXzqZheHxNYQZzrww+rdXs1hQ35ke6Uc+XYVAAxUW36
NY5DLickQ+nVqT4/T/dWfdNziXAMle4gELxEc8bxcxROPtCxk8z6l/9M78nIuuUi
1Fn/EAMXpv9/A6E6crn9RcubRmp8Jh8wYVcH05qD9e/nIeqseWlOSf7NhKawL02T
8U6w3UCmxrUp/CP1K9GmUCbzoDNzehseMeR2I4rGKa1YlYaL5rl6rd9hGT9ywgjE
7QMcZiT2tSVRBSLCv8yompJVz/G+LrDyP7T8JrfgGyieKE50as24DXLOWXnWnHLB
EQxzTsmRAgMBAAECggEADv1Tb4JNz3QvyedwwA4yi/7jNwYtyvYm+mPz+xew1pop
86RusQUwA3/0o8tTxWfLWQzxq17Gyr3v9idG+HT89uHfd05FMhge4a1FXhtCgqtZ
mzEGdW27FjOrEa/6jIiqWYBphtAembL3sOXsa711MwVHegTExloz+aU2kuPRqfyf
ATDXPYw1J/sLd3Xamr3Y0LJ/yiVR3M1WAkUX3myjbmss5O+yfK4HiY7xXCxF218C
YVb3r7caIgUruV/D1hQncIZR95c2duBoQDcXoOEvQ67NhbDMTeixXTbAaBYP7Ypq
f87Ljdl3+KuX9XTx+RRGUFn9uQOECYCF5gEtU+fGYQKBgQD/+nKqweDiSrjqwmT5
DC0h9WBVEce08fumtfTC+1dlwv9ZNa8ns+UZWg7EBeKDeaE7s+Ru11KOq23allNV
gFV3Kqw4XOdI6g3TDW3XgYwtQ77JPTOymaQIEb6pxLzRrfwWPccSKIitlP0ff7VU
0WuBJWxZ2g6bPOw+egigbay4+QKBgQDSIMdPFsDh/X3E3NRD6lrQCXm/YWOICN/q
rG3SuTDisGMBmtsaLzvr7PVBk9QUd9eggwE2udlJyv73DtT5ZZUsQ7M37SlWGfHT
8pyb4OQy1xGPmc9NkolXlIaRi33WzCXKMP3w5a9MIHz4m6X0WgZQO/4XnM+I0/hY
7vUvbsgTWQKBgQCzSGk5ebMVKzqaie6Ik+OkbiS7UEmsTPNxTu2QBtOunUWU6Mm1
qASknfPLjUeZx/2KQDOVAlB7RkwZlcHmF41EemnGzCLdabinAjfVgZF5PoKIlcn4
pC1DzZHZe8a3oQD3Xutnp2YbFUe34Q1Sy55dBKX/xH8IcUIRfA1At7AKmQKBgQDL
q6/kLfbJVa4pOa6ZIbfiO70BToFt4sQ/L+DHRm9m2ncsoA/NQok/NY/Hf2UqbbrY
PwLXK668gwE9MOgn0FmV7QzyoXLWnRE0Uc2QnZwy1xmTag9wbh+nfzQsMNvJblkW
sQQDEm4mSLs5MYza2sOR04SHGJxkUKlAcmW/Ew7lCQKBgERBap56h1tVY2m+8gyz
Euiyfuo1Nk6/jygP9jL2bpmbAM2YgUrio/0YNgVDIVpWdRtQ2DI2WwuD9M7o6Fed
CekE2y43oejWM38zbrMChkwnWoNYddrSE+LoQhHUntzDkdM0JG6M6rvIgzEBU1Ko
Avd7QBCxvqXU+7acaZ2xxaV4
-----END PRIVATE KEY-----"#;

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: String) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    fn sample_env(issuer: String) -> EnvConfig {
        EnvConfig {
            graphql_url: "https://example.invalid/graphql".to_string(),
            auth: AuthConfig {
                issuer,
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        }
    }

    fn mock_oidc(server: &mut Server, issuer: &str, expected_calls: usize) -> (Mock, Mock) {
        let discovery = server
            .mock("GET", "/.well-known/openid-configuration")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "issuer": issuer,
                    "jwks_uri": format!("{}/jwks", server.url())
                })
                .to_string(),
            )
            .expect(expected_calls)
            .create();

        let jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "keys": [{
                        "kid": TEST_KID,
                        "kty": "RSA",
                        "use": "sig",
                        "alg": "RS256",
                        "n": TEST_RSA_N,
                        "e": TEST_RSA_E
                    }]
                })
                .to_string(),
            )
            .expect(expected_calls)
            .create();

        (discovery, jwks)
    }

    fn make_verified_token(env_cfg: &EnvConfig, extra_claims: Value) -> String {
        let mut claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": env_cfg.auth.audience,
            "exp": current_epoch_seconds() + 3600,
            "person_id": "p-1"
        });

        if let Some(extra) = extra_claims.as_object() {
            let base = claims
                .as_object_mut()
                .expect("base claims should always be object");
            for (key, value) in extra {
                base.insert(key.clone(), value.clone());
            }
        }

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("test private key"),
        )
        .expect("rs256 token encoding")
    }

    fn sample_session(
        env_cfg: &EnvConfig,
        expires_at: Option<i64>,
        refresh_token: Option<&str>,
    ) -> Session {
        Session {
            access_token: make_verified_token(env_cfg, json!({ "exp": 9_999_999_999_i64 })),
            refresh_token: refresh_token.map(ToString::to_string),
            id_token: Some("id-token".to_string()),
            expires_at,
            person_id: "p-1".to_string(),
            source: LoginSource::DeviceCode,
        }
    }

    fn test_dpop() -> AuthDpopContext {
        AuthDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([3_u8; 32]).expect("fixed dpop key"),
        )
    }

    fn runtime_dpop_options() -> DpopRuntimeOptions {
        DpopRuntimeOptions {
            key_backend: crate::config::DpopKeyBackend::File,
            pkcs11: None,
        }
    }

    fn ensure_runtime_dpop_key() {
        DpopKeyMaterial::load_or_create_for_options(&runtime_dpop_options())
            .expect("create runtime dpop key");
    }

    fn current_runtime_dpop_thumbprint() -> String {
        DpopKeyMaterial::load_existing_for_options(&runtime_dpop_options())
            .expect("load runtime dpop key")
            .jwk_thumbprint()
            .expect("runtime dpop thumbprint")
    }

    fn stored_session_with_thumbprint(
        env_cfg: &EnvConfig,
        expires_at: Option<i64>,
        refresh_token: Option<&str>,
        dpop_jwk_thumbprint: Option<&str>,
    ) -> StoredSession {
        StoredSession {
            env: TargetEnv::Prod,
            session: sample_session(env_cfg, expires_at, refresh_token),
            dpop_jwk_thumbprint: dpop_jwk_thumbprint.map(ToString::to_string),
            mode: None,
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedProgressEvent {
        BrowserPending(u64),
        BrowserSlowDown(u64),
        BrowserWait(Duration),
        SecondFactorNotice,
        SecondFactorWait(Duration),
        Finish,
    }

    struct RecordingLoginProgress {
        events: Vec<RecordedProgressEvent>,
    }

    impl RecordingLoginProgress {
        fn new() -> Self {
            Self { events: Vec::new() }
        }
    }

    impl LoginProgressSink for RecordingLoginProgress {
        fn note_browser_confirmation_pending(&mut self, pending_polls: u64) -> Result<()> {
            self.events
                .push(RecordedProgressEvent::BrowserPending(pending_polls));
            Ok(())
        }

        fn note_browser_confirmation_slow_down(&mut self, interval_secs: u64) -> Result<()> {
            self.events
                .push(RecordedProgressEvent::BrowserSlowDown(interval_secs));
            Ok(())
        }

        fn wait_for_browser_confirmation(&mut self, duration: Duration) -> Result<()> {
            self.events
                .push(RecordedProgressEvent::BrowserWait(duration));
            Ok(())
        }

        fn note_second_factor_wait(&mut self) -> Result<()> {
            self.events.push(RecordedProgressEvent::SecondFactorNotice);
            Ok(())
        }

        fn wait_for_second_factor(&mut self, duration: Duration) -> Result<()> {
            self.events
                .push(RecordedProgressEvent::SecondFactorWait(duration));
            Ok(())
        }

        fn finish(&mut self) -> Result<()> {
            self.events.push(RecordedProgressEvent::Finish);
            Ok(())
        }
    }

    #[test]
    fn refresh_if_needed_returns_none_for_fresh_token() {
        let env_cfg = sample_env("https://issuer.invalid".to_string());
        let session = sample_session(&env_cfg, Some(5_000), Some("refresh-1"));
        let refreshed =
            refresh_session_if_needed_at(&env_cfg, &session, 1_000, &test_dpop()).unwrap();
        assert!(refreshed.is_none());
    }

    #[test]
    fn refresh_if_needed_errors_when_expired_without_refresh_token() {
        let env_cfg = sample_env("https://issuer.invalid".to_string());
        let session = sample_session(&env_cfg, Some(900), None);
        let err =
            refresh_session_if_needed_at(&env_cfg, &session, 1_000, &test_dpop()).unwrap_err();
        assert!(
            err.to_string()
                .contains("expired and no refresh token is available")
        );
    }

    #[test]
    fn refresh_if_needed_requires_existing_dpop_key_for_authenticated_session() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let env_cfg = sample_env("https://issuer.invalid".to_string());
        let session = sample_session(&env_cfg, Some(1_001), Some("refresh-1"));

        let err = refresh_session_if_needed(&env_cfg, &session, &runtime_dpop_options())
            .expect_err("missing dpop key should fail before refresh");

        assert_eq!(err.to_string(), DPOP_SESSION_KEY_RELOGIN_MESSAGE);
    }

    #[test]
    fn validate_stored_session_dpop_binding_accepts_matching_thumbprint() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let env_cfg = sample_env("https://issuer.invalid".to_string());
        ensure_runtime_dpop_key();
        let runtime_thumbprint = current_runtime_dpop_thumbprint();

        let stored = stored_session_with_thumbprint(
            &env_cfg,
            Some(1_001),
            Some("refresh-1"),
            Some(runtime_thumbprint.as_str()),
        );

        let thumbprint =
            validate_stored_session_dpop_binding(&stored, &runtime_dpop_options()).expect("valid");

        assert_eq!(thumbprint, runtime_thumbprint);
    }

    #[test]
    fn validate_stored_session_dpop_binding_rejects_missing_thumbprint() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let env_cfg = sample_env("https://issuer.invalid".to_string());
        ensure_runtime_dpop_key();

        let stored = stored_session_with_thumbprint(&env_cfg, Some(1_001), Some("refresh-1"), None);
        let err = validate_stored_session_dpop_binding(&stored, &runtime_dpop_options())
            .expect_err("missing thumbprint should require relogin");

        assert_eq!(err.to_string(), DPOP_SESSION_KEY_RELOGIN_MESSAGE);
    }

    #[test]
    fn validate_stored_session_dpop_binding_rejects_mismatched_thumbprint() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let env_cfg = sample_env("https://issuer.invalid".to_string());
        ensure_runtime_dpop_key();

        let stored = stored_session_with_thumbprint(
            &env_cfg,
            Some(1_001),
            Some("refresh-1"),
            Some("different-thumbprint"),
        );
        let err = validate_stored_session_dpop_binding(&stored, &runtime_dpop_options())
            .expect_err("mismatched thumbprint should require relogin");

        assert_eq!(err.to_string(), DPOP_SESSION_KEY_RELOGIN_MESSAGE);
    }

    #[test]
    fn validate_stored_session_dpop_binding_rejects_missing_existing_key() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let env_cfg = sample_env("https://issuer.invalid".to_string());

        let stored = stored_session_with_thumbprint(
            &env_cfg,
            Some(1_001),
            Some("refresh-1"),
            Some("thumbprint-1"),
        );
        let err = validate_stored_session_dpop_binding(&stored, &runtime_dpop_options())
            .expect_err("missing key should require relogin");

        assert_eq!(err.to_string(), DPOP_SESSION_KEY_RELOGIN_MESSAGE);
    }

    #[test]
    fn refresh_if_needed_uses_replacement_refresh_token() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
        let _m = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"access_token":"{}","refresh_token":"refresh-2","expires_in":3600}}"#,
                make_verified_token(&env_cfg, json!({}))
            ))
            .create();

        let session = sample_session(&env_cfg, Some(1_001), Some("refresh-1"));
        let refreshed = refresh_session_if_needed_at(&env_cfg, &session, 1_000, &test_dpop())
            .unwrap()
            .expect("refresh expected");

        assert_eq!(refreshed.person_id, "p-1");
        assert_eq!(refreshed.refresh_token.as_deref(), Some("refresh-2"));
        assert!(refreshed.expires_at.unwrap_or_default() > 1_000);
    }

    #[test]
    fn refresh_if_needed_errors_when_replacement_refresh_token_is_missing() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
        let _m = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"access_token":"{}","expires_in":3600}}"#,
                make_verified_token(&env_cfg, json!({}))
            ))
            .create();

        let session = sample_session(&env_cfg, Some(1_001), Some("refresh-1"));
        let err =
            refresh_session_if_needed_at(&env_cfg, &session, 1_000, &test_dpop()).unwrap_err();

        assert!(
            err.to_string().contains(REFRESH_RELOGIN_REQUIRED_PREFIX),
            "{err:#}"
        );
        assert!(
            err.to_string()
                .contains("no usable replacement refresh token was returned")
        );
    }

    #[test]
    fn refresh_if_needed_errors_when_replacement_refresh_token_is_blank() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
        let _m = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"access_token":"{}","refresh_token":"   ","expires_in":3600}}"#,
                make_verified_token(&env_cfg, json!({}))
            ))
            .create();

        let session = sample_session(&env_cfg, Some(1_001), Some("refresh-1"));
        let err =
            refresh_session_if_needed_at(&env_cfg, &session, 1_000, &test_dpop()).unwrap_err();

        assert!(err.to_string().contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
        assert!(
            err.to_string()
                .contains("no usable replacement refresh token was returned")
        );
    }

    #[test]
    fn refresh_if_needed_errors_when_refreshed_access_token_fails_verification() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
        let _m = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"access_token":"not-a-jwt","refresh_token":"refresh-2","expires_in":3600}"#,
            )
            .create();

        let session = sample_session(&env_cfg, Some(1_001), Some("refresh-1"));
        let err =
            refresh_session_if_needed_at(&env_cfg, &session, 1_000, &test_dpop()).unwrap_err();

        assert!(err.to_string().contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
        assert!(
            err.to_string()
                .contains("refreshed access token failed verification")
        );
    }

    #[test]
    fn refresh_if_needed_passes_session_id_claim_when_present() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
        let _m = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
                Matcher::UrlEncoded("session_id".into(), "sid-123".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"access_token":"{}","refresh_token":"refresh-2","expires_in":3600}}"#,
                make_verified_token(&env_cfg, json!({}))
            ))
            .create();

        let session = Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({
                    "https://de.scalable.capital/session_id": "sid-123",
                    "exp": 9_999_999_999_i64
                }),
            ),
            refresh_token: Some("refresh-1".to_string()),
            id_token: Some("id-token".to_string()),
            expires_at: Some(1_001),
            person_id: "p-1".to_string(),
            source: LoginSource::DeviceCode,
        };

        let refreshed = refresh_session_if_needed_at(&env_cfg, &session, 1_000, &test_dpop())
            .unwrap()
            .expect("refresh expected");
        assert_eq!(refreshed.person_id, "p-1");
    }

    #[test]
    fn logout_refresh_revoke_posts_token_and_client_id() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let revoke = server
            .mock("POST", "/oauth/revoke")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("token".into(), "refresh-1".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            ]))
            .with_status(200)
            .expect(1)
            .create();

        let env_cfg = sample_env(server.url());
        revoke_refresh_token_on_logout(&env_cfg, "refresh-1", &runtime_dpop_options())
            .expect("refresh revoke should work");

        revoke.assert();
    }

    #[test]
    fn logout_access_revoke_calls_graphql_mutation() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let access_token = "access-token-1";
        let revoke = server
            .mock("POST", "/")
            .match_body(Matcher::AllOf(vec![
                PartialJson(serde_json::json!({
                    "operation_name": "revokeAuthAccessToken",
                    "variables": {
                        "input": {
                            "accessToken": access_token
                        }
                    }
                })),
                Matcher::Regex("revokeAuthAccessToken".to_string()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"revokeAuthAccessToken":true}}"#)
            .expect(1)
            .create();

        let env_cfg = sample_env("https://issuer.invalid".to_string());
        let env_cfg = EnvConfig {
            graphql_url: server.url(),
            auth: env_cfg.auth,
        };
        revoke_access_token_on_logout(&env_cfg, access_token, None, &runtime_dpop_options())
            .expect("access revoke mutation should succeed");

        revoke.assert();
    }

    #[test]
    fn logout_best_effort_skips_oauth_revoke_without_refresh_token() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let session = sample_session(&env_cfg, Some(1_500), None);

        let gql_revoke = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("revokeAuthAccessToken".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"revokeAuthAccessToken":true}}"#)
            .expect(1)
            .create();

        let oauth_revoke = server.mock("POST", "/oauth/revoke").expect(0).create();

        let env_cfg = EnvConfig {
            graphql_url: server.url(),
            auth: env_cfg.auth,
        };
        revoke_tokens_on_logout_best_effort(&env_cfg, &session, None, &runtime_dpop_options());

        gql_revoke.assert();
        oauth_revoke.assert();
    }

    #[test]
    fn logout_best_effort_continues_when_refresh_revoke_fails() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let session = sample_session(&env_cfg, Some(1_500), Some("refresh-1"));

        let oauth_revoke = server
            .mock("POST", "/oauth/revoke")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("token".into(), "refresh-1".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            ]))
            .with_status(500)
            .expect(1)
            .create();

        let gql_revoke = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("revokeAuthAccessToken".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"revokeAuthAccessToken":true}}"#)
            .expect(1)
            .create();

        let env_cfg = EnvConfig {
            graphql_url: server.url(),
            auth: env_cfg.auth,
        };
        revoke_tokens_on_logout_best_effort(&env_cfg, &session, None, &runtime_dpop_options());

        oauth_revoke.assert();
        gql_revoke.assert();
    }

    #[test]
    fn device_code_request_success_parses_response() {
        let mut server = Server::new();
        let _m = server
            .mock("POST", "/oauth/device/code")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("audience".into(), "aud".into()),
                Matcher::UrlEncoded("scope".into(), AUTH_SCOPE.into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"device_code":"dc","user_code":"uc","verification_uri":"https://verify","expires_in":600,"interval":1}"#,
            )
            .create();

        let env_cfg = sample_env(server.url());
        let client = Client::new();
        let response = request_device_code(&client, &env_cfg, &test_dpop()).unwrap();
        assert_eq!(response.device_code, "dc");
        assert_eq!(response.user_code, "uc");
    }

    #[test]
    fn local_file_storage_warning_detected_from_diagnostics() {
        assert!(should_print_local_file_storage_warning(
            &StorageBackendDiagnostics {
                configured_backend: "auto".to_string(),
                effective_backend: "file".to_string(),
                fallback_reason: Some("keyring unavailable".to_string()),
            }
        ));
    }

    #[test]
    fn local_file_storage_warning_skipped_for_keyring_diagnostics() {
        assert!(!should_print_local_file_storage_warning(
            &StorageBackendDiagnostics {
                configured_backend: "auto".to_string(),
                effective_backend: "keyring".to_string(),
                fallback_reason: None,
            }
        ));
    }

    fn sample_device_code_response(verification_uri_complete: Option<&str>) -> DeviceCodeResponse {
        DeviceCodeResponse {
            device_code: "device-code".to_string(),
            user_code: "VXWZ-JZDW".to_string(),
            verification_uri: "https://verify".to_string(),
            verification_uri_complete: verification_uri_complete.map(ToString::to_string),
            expires_in: 600,
            interval: Some(5),
        }
    }

    #[test]
    fn device_login_prompt_formats_complete_url_with_stdout_warning_and_bold_emphasis() {
        let prompt = format_device_login_prompt(
            &sample_device_code_response(Some("https://verify/activate?user_code=VXWZ-JZDW")),
            true,
            true,
        );

        assert_eq!(
            prompt,
            "\u{1b}[1mWarning\u{1b}[0m: session secrets are stored in local files. Prefer keyring-backed session storage on this device.\n\n\
Open this URL:\n\
https://verify/activate?user_code=VXWZ-JZDW\n\n\
Verify the code \u{1b}[1mVXWZ-JZDW\u{1b}[0m in your browser.\n\n"
        );
    }

    #[test]
    fn device_login_prompt_formats_base_url_without_warning_in_plain_text() {
        let prompt = format_device_login_prompt(&sample_device_code_response(None), false, false);

        assert_eq!(
            prompt,
            "Open this URL:\nhttps://verify\n\nVerify the code VXWZ-JZDW in your browser.\n\n"
        );
        assert!(!prompt.contains(ANSI_BOLD_PREFIX));
    }

    #[test]
    fn local_file_storage_warning_is_plain_when_terminal_styling_is_disabled() {
        let warning = format_local_file_storage_warning(false);

        assert_eq!(
            warning,
            "Warning: session secrets are stored in local files. Prefer keyring-backed session storage on this device."
        );
        assert!(!warning.contains(ANSI_BOLD_PREFIX));
    }

    #[test]
    fn device_poll_error_contains_grant_hint() {
        let mut server = Server::new();
        let _m = server
            .mock("POST", "/oauth/token")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"error":"unauthorized_client","error_description":"Grant type not allowed for this client"}"#,
            )
            .create();

        let env_cfg = sample_env(server.url());
        let client = Client::new();
        let err =
            poll_device_token_once(&client, &env_cfg, "device-code", &test_dpop()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("grant type"));
        assert!(msg.contains("Device Code"));
    }

    #[test]
    fn send_oauth_form_adds_dpop_header_when_enabled() {
        let mut server = Server::new();
        let _m = server
            .mock("POST", "/oauth/token")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .create();

        let client = Client::new();
        let mut form = HashMap::new();
        form.insert("grant_type", "refresh_token");
        form.insert("client_id", "client-id");
        form.insert("refresh_token", "refresh-1");

        let dpop = AuthDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([9_u8; 32]).expect("fixed dpop key"),
        );

        let response = send_oauth_form(
            &client,
            &format!("{}/oauth/token", server.url()),
            &form,
            "Token refresh",
            &dpop,
        )
        .expect("request should succeed");

        assert!(response.status().is_success());
    }

    #[test]
    fn send_oauth_form_retries_once_when_dpop_nonce_is_challenged() {
        let mut server = Server::new();
        let first = server
            .mock("POST", "/oauth/token")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(401)
            .with_header("DPoP-Nonce", "nonce-1")
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"use_dpop_nonce","error_description":"Provide DPoP nonce"}"#)
            .expect(1)
            .create();
        let second = server
            .mock("POST", "/oauth/token")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .expect(1)
            .create();

        let client = Client::new();
        let mut form = HashMap::new();
        form.insert("grant_type", "refresh_token");
        form.insert("client_id", "client-id");
        form.insert("refresh_token", "refresh-1");
        let dpop = AuthDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([4_u8; 32]).expect("fixed dpop key"),
        );

        let response = send_oauth_form(
            &client,
            &format!("{}/oauth/token", server.url()),
            &form,
            "Token refresh",
            &dpop,
        )
        .expect("request should succeed on retry");

        assert!(response.status().is_success());
        first.assert();
        second.assert();
    }

    #[test]
    fn send_oauth_form_adds_dpop_clock_hint_when_proof_is_rejected() {
        let mut server = Server::new();
        let first = server
            .mock("POST", "/oauth/token")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"error":"invalid_dpop_proof","error_description":"DPoP proof was rejected"}"#,
            )
            .expect(1)
            .create();

        let client = Client::new();
        let mut form = HashMap::new();
        form.insert("grant_type", "refresh_token");
        form.insert("client_id", "client-id");
        form.insert("refresh_token", "refresh-1");
        let dpop = AuthDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([8_u8; 32]).expect("fixed dpop key"),
        );

        let err = send_oauth_form(
            &client,
            &format!("{}/oauth/token", server.url()),
            &form,
            "Token refresh",
            &dpop,
        )
        .expect_err("request should fail with dpop troubleshooting hint");

        let msg = err.to_string();
        assert!(msg.contains("Token refresh failed"));
        assert!(msg.contains("system clock is in sync"));
        first.assert();
    }

    #[test]
    fn send_oauth_form_error_omits_raw_error_description() {
        let mut server = Server::new();
        let marker = "LEAK_SECRET_AUTH_3";
        let _m = server
            .mock("POST", "/oauth/token")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"error":"unauthorized_client","error_description":"grant blocked: {marker}"}}"#
            ))
            .create();

        let client = Client::new();
        let mut form = HashMap::new();
        form.insert("grant_type", "refresh_token");
        form.insert("client_id", "client-id");
        form.insert("refresh_token", "refresh-1");

        let err = send_oauth_form(
            &client,
            &format!("{}/oauth/token", server.url()),
            &form,
            "Token refresh",
            &test_dpop(),
        )
        .expect_err("request should fail");

        let msg = err.to_string();
        assert!(msg.contains("Token refresh failed (OAuth 403 - unauthorized_client)"));
        assert!(!msg.contains(marker));
    }

    #[test]
    fn device_wait_message_is_rate_limited() {
        assert!(should_print_device_wait_message(1));
        assert!(!should_print_device_wait_message(2));
        assert!(should_print_device_wait_message(3));
        assert!(!should_print_device_wait_message(4));
        assert!(!should_print_device_wait_message(5));
        assert!(should_print_device_wait_message(6));
    }

    #[test]
    fn interactive_login_status_disabled_for_non_terminal_output() {
        assert!(!should_use_interactive_login_status(false));
    }

    #[test]
    fn interactive_login_status_disabled_for_dumb_term() {
        let _lock = crate::lock_test_env();
        let _guard = EnvGuard::set("TERM", "dumb".to_string());
        assert!(!should_use_interactive_login_status(true));
    }

    #[test]
    fn interactive_login_status_enabled_for_tty_with_normal_term() {
        let _lock = crate::lock_test_env();
        let _guard = EnvGuard::set("TERM", "xterm-256color".to_string());
        assert!(should_use_interactive_login_status(true));
    }

    #[test]
    fn plain_text_progress_keeps_browser_message_cadence_and_second_factor_wording() {
        let mut progress = TerminalLoginProgress::new(Vec::new(), false);

        progress
            .note_browser_confirmation_pending(1)
            .expect("first browser wait should render");
        progress
            .note_browser_confirmation_pending(2)
            .expect("second browser wait should stay quiet");
        progress
            .note_browser_confirmation_slow_down(7)
            .expect("slow down should render");
        progress
            .note_second_factor_wait()
            .expect("second factor notice should render");
        progress
            .note_second_factor_wait()
            .expect("duplicate second factor notice should stay quiet");

        let output = String::from_utf8(progress.into_inner()).expect("utf8 output");
        assert!(output.contains("Waiting for browser confirmation...\n"));
        assert!(
            output.contains(
                "Waiting for browser confirmation... (Auth server asked to slow down; polling every 7s)\n"
            )
        );
        assert!(output.contains("Waiting for second factor approval on your linked device...\n"));
        assert_eq!(
            output
                .matches("Waiting for second factor approval on your linked device...")
                .count(),
            1
        );
    }

    #[test]
    fn interactive_progress_renders_browser_and_second_factor_status_lines() {
        let mut progress = TerminalLoginProgress::new(Vec::new(), true);

        progress
            .note_browser_confirmation_pending(1)
            .expect("browser status should render");
        progress
            .note_second_factor_wait()
            .expect("second factor status should render");
        progress.finish().expect("finish should clear the line");

        let output = String::from_utf8(progress.into_inner()).expect("utf8 output");
        assert!(output.contains(BROWSER_CONFIRMATION_WAIT_MESSAGE));
        assert!(output.contains(SECOND_FACTOR_WAIT_MESSAGE));
        assert!(output.contains('\r'));
    }

    #[test]
    fn post_login_mfa_error_is_sanitized() {
        let marker = "LEAK_TRUSTED_DEVICE_FAILURE";
        let err = anyhow::anyhow!(
            "GraphQL returned errors for Validate2faOnLogin (code: UNAUTHENTICATED): {marker}"
        );
        let msg = sanitize_post_login_mfa_error(err).to_string();
        assert_eq!(msg, TRUSTED_DEVICE_2FA_FAILURE_MESSAGE);
        assert!(!msg.contains(marker));
    }

    fn login_with_trusted_device_2fa_validation_failure(
        env: TargetEnv,
    ) -> Result<(SessionManager, tempfile::TempDir)> {
        let _lock = crate::lock_test_env();
        let mut server = Server::new();
        let env_cfg = EnvConfig {
            graphql_url: format!("{}/graphql", server.url()),
            auth: AuthConfig {
                issuer: server.url(),
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        };
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
        let access_token = make_verified_token(&env_cfg, json!({}));
        let _device_code = server
            .mock("POST", "/oauth/device/code")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("audience".into(), "aud".into()),
                Matcher::UrlEncoded("scope".into(), AUTH_SCOPE.into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"device_code":"dc","user_code":"uc","verification_uri":"https://verify","expires_in":600,"interval":1}"#,
            )
            .expect(1)
            .create();
        let _token = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded(
                    "grant_type".into(),
                    "urn:ietf:params:oauth:grant-type:device_code".into(),
                ),
                Matcher::UrlEncoded("device_code".into(), "dc".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"access_token":"{access_token}","expires_in":3600}}"#
            ))
            .expect(1)
            .create();
        let _is_enabled = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Is2faOnLoginEnabled",
                "variables": {
                    "input": {
                        "userId": "p-1"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"is2faOnLoginEnabled":{"enabled":true,"hasApprovedSession":false}}}"#,
            )
            .expect(1)
            .create();
        let _start = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Start2faOnLogin",
                "variables": {
                    "input": {
                        "userId": "p-1",
                        "deviceName": "CLI",
                        "deviceType": "CLI"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"start2faOnLogin":{"mfaSessionId":"mfa-1"}}}"#)
            .expect(1)
            .create();
        let _validate = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Validate2faOnLogin",
                "variables": {
                    "input": {
                        "userId": "p-1",
                        "mfaSessionId": "mfa-1"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"errors":[{"extensions":{"code":"UNAUTHENTICATED"},"message":"LEAK_ME","path":["validate2faOnLogin"]}]}"#,
            )
            .expect(1)
            .create();

        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let store = crate::session::StorageBackend::File(
            crate::session::FileStore::new(tmp.path().to_path_buf()).expect("file store"),
        );
        let mut session_manager = SessionManager::with_store(store);

        let _backend = login_with_device_code(
            &mut session_manager,
            env,
            &env_cfg,
            None,
            &runtime_dpop_options(),
        )?;
        Ok((session_manager, tmp))
    }

    fn record_post_login_mfa_progress(has_approved_session: bool) -> Vec<RecordedProgressEvent> {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let env_cfg = EnvConfig {
            graphql_url: format!("{}/graphql", server.url()),
            auth: AuthConfig {
                issuer: server.url(),
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        };
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
        let access_token = make_verified_token(&env_cfg, json!({}));
        let _is_enabled = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Is2faOnLoginEnabled",
                "variables": {
                    "input": {
                        "userId": "p-1"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"data":{{"is2faOnLoginEnabled":{{"enabled":true,"hasApprovedSession":{has_approved_session}}}}}}}"#
            ))
            .expect(1)
            .create();
        let _start = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Start2faOnLogin",
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"start2faOnLogin":{"mfaSessionId":"mfa-1"}}}"#)
            .expect(usize::from(!has_approved_session))
            .create();
        let _validate = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Validate2faOnLogin",
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"validate2faOnLogin":{"status":"SUCCESS"}}}"#)
            .expect(usize::from(!has_approved_session))
            .create();

        let mut progress = RecordingLoginProgress::new();
        handle_post_login_mfa(
            &mut progress,
            &env_cfg,
            &access_token,
            "p-1",
            &runtime_dpop_options(),
        )
        .expect("post-login mfa should succeed");

        progress.events
    }

    fn login_with_existing_approved_trusted_device_session(
        env: TargetEnv,
        session_mode: Option<SessionMode>,
    ) -> Result<(SessionManager, tempfile::TempDir)> {
        let _lock = crate::lock_test_env();
        let mut server = Server::new();
        let env_cfg = EnvConfig {
            graphql_url: format!("{}/graphql", server.url()),
            auth: AuthConfig {
                issuer: server.url(),
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        };
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
        let access_token = make_verified_token(&env_cfg, json!({}));
        let _device_code = server
            .mock("POST", "/oauth/device/code")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("audience".into(), "aud".into()),
                Matcher::UrlEncoded("scope".into(), AUTH_SCOPE.into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"device_code":"dc","user_code":"uc","verification_uri":"https://verify","expires_in":600,"interval":1}"#,
            )
            .expect(1)
            .create();
        let _token = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded(
                    "grant_type".into(),
                    "urn:ietf:params:oauth:grant-type:device_code".into(),
                ),
                Matcher::UrlEncoded("device_code".into(), "dc".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"access_token":"{access_token}","expires_in":3600}}"#
            ))
            .expect(1)
            .create();
        let _is_enabled = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Is2faOnLoginEnabled",
                "variables": {
                    "input": {
                        "userId": "p-1"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"is2faOnLoginEnabled":{"enabled":true,"hasApprovedSession":true}}}"#,
            )
            .expect(1)
            .create();
        let _start = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Start2faOnLogin",
            })))
            .expect(0)
            .create();
        let _validate = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Validate2faOnLogin",
            })))
            .expect(0)
            .create();

        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let store = crate::session::StorageBackend::File(
            crate::session::FileStore::new(tmp.path().to_path_buf()).expect("file store"),
        );
        let mut session_manager = SessionManager::with_store(store);

        let _backend = login_with_device_code(
            &mut session_manager,
            env,
            &env_cfg,
            session_mode,
            &runtime_dpop_options(),
        )?;
        Ok((session_manager, tmp))
    }

    #[test]
    fn login_with_device_code_fails_when_trusted_device_2fa_verification_fails_in_prod() {
        let err = login_with_trusted_device_2fa_validation_failure(TargetEnv::Prod)
            .err()
            .expect("login should fail");
        let msg = err.to_string();
        assert_eq!(msg, TRUSTED_DEVICE_2FA_FAILURE_MESSAGE);
        assert!(!msg.contains("LEAK_ME"));
    }

    #[test]
    fn login_with_device_code_fails_when_trusted_device_2fa_verification_fails_in_dev() {
        let err = login_with_trusted_device_2fa_validation_failure(TargetEnv::Dev)
            .err()
            .expect("login should fail");
        let msg = err.to_string();
        assert_eq!(msg, TRUSTED_DEVICE_2FA_FAILURE_MESSAGE);
        assert!(!msg.contains("LEAK_ME"));
    }

    #[test]
    fn post_login_mfa_skips_second_factor_progress_for_approved_session() {
        let events = record_post_login_mfa_progress(true);
        assert!(!events.contains(&RecordedProgressEvent::SecondFactorNotice));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RecordedProgressEvent::SecondFactorWait(_)))
        );
    }

    #[test]
    fn post_login_mfa_emits_second_factor_progress_for_new_challenge() {
        let events = record_post_login_mfa_progress(false);
        assert!(events.contains(&RecordedProgressEvent::SecondFactorNotice));
    }

    #[test]
    fn login_with_device_code_skips_trusted_device_challenge_for_approved_session() {
        let (session_manager, _tmp) =
            login_with_existing_approved_trusted_device_session(TargetEnv::Prod, None)
                .expect("login should succeed");

        assert!(
            session_manager
                .load_active()
                .expect("load session")
                .is_some_and(|stored| stored.env == TargetEnv::Prod)
        );
        let stored = session_manager
            .load_active()
            .expect("load session")
            .expect("stored session");
        assert!(stored.dpop_jwk_thumbprint.is_some());
        assert_eq!(stored.mode, None);
    }

    #[test]
    fn login_with_device_code_persists_local_read_only_mode() {
        let (session_manager, _tmp) = login_with_existing_approved_trusted_device_session(
            TargetEnv::Prod,
            Some(SessionMode::LocalReadOnly),
        )
        .expect("login should succeed");

        let stored = session_manager
            .load_active()
            .expect("load session")
            .expect("stored session");
        assert_eq!(stored.mode, Some(SessionMode::LocalReadOnly));
    }
}
