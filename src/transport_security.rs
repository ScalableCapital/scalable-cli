use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::redirect::Policy;
use url::Url;

use crate::config::EnvConfig;

pub const RUNTIME_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
pub const UPDATE_CHECK_HTTP_TIMEOUT: Duration = Duration::from_secs(1);
pub const CLI_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

pub fn validate_env_transport_security(env_cfg: &EnvConfig) -> Result<()> {
    validate_https_url(&env_cfg.graphql_url, "graphql_url")?;
    validate_https_url(&env_cfg.auth.issuer, "auth.issuer")?;
    Ok(())
}

pub fn validate_https_url(raw_url: &str, field_name: &str) -> Result<Url> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        bail!("{field_name} must not be empty");
    }

    let parsed = Url::parse(trimmed)
        .with_context(|| format!("{field_name} is not a valid URL: {trimmed}"))?;

    if is_allowed_transport_scheme(&parsed) {
        return Ok(parsed);
    }

    bail!(
        "{field_name} must use https (got scheme '{}' for '{}')",
        parsed.scheme(),
        trimmed
    )
}

pub fn build_blocking_client_https_only_with_timeout(timeout: Duration) -> Result<Client> {
    configure_client_builder(
        Client::builder()
            .user_agent(CLI_USER_AGENT)
            .timeout(timeout),
    )
    .build()
    .context("Failed to build HTTPS-only HTTP client")
}

pub fn build_update_check_client() -> Result<Client> {
    configure_client_builder(
        Client::builder()
            .user_agent(CLI_USER_AGENT)
            .timeout(UPDATE_CHECK_HTTP_TIMEOUT)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy(),
    )
    .build()
    .context("Failed to build update-check HTTP client")
}

fn configure_client_builder(builder: ClientBuilder) -> ClientBuilder {
    #[cfg(not(test))]
    {
        builder.https_only(true)
    }

    #[cfg(test)]
    {
        // Unit tests use loopback HTTP mock servers (mockito), so HTTPS-only is disabled in test builds.
        builder
    }
}

fn is_allowed_transport_scheme(url: &Url) -> bool {
    if url.scheme() == "https" {
        return true;
    }

    #[cfg(test)]
    {
        is_loopback_http_for_tests(url)
    }

    #[cfg(not(test))]
    {
        false
    }
}

#[cfg(test)]
fn is_loopback_http_for_tests(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }

    url.host_str()
        .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "::1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, EnvConfig};
    use mockito::Server;
    use std::env;
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use std::thread;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl Into<OsString>) -> Self {
            let previous = env::var_os(key);
            unsafe {
                env::set_var(key, value.into());
            }
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = env::var_os(key);
            unsafe {
                env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => env::set_var(self.key, value),
                    None => env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn timeout_client_fails_fast_on_stalled_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request_buf = [0_u8; 1024];
            let _ = stream.read(&mut request_buf);
            thread::sleep(Duration::from_millis(300));
        });

        let client = build_blocking_client_https_only_with_timeout(Duration::from_millis(50))
            .expect("client");
        let err = client
            .get(format!("http://{addr}/"))
            .send()
            .expect_err("request should time out");

        let timeout_like = err.is_timeout() || err.to_string().to_lowercase().contains("timed out");
        assert!(timeout_like, "expected timeout-like error, got: {err}");

        worker.join().expect("worker join");
    }

    #[test]
    fn client_sends_cli_user_agent_header() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buf).expect("read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .expect("write response");
            String::from_utf8(request).expect("utf8 request")
        });

        let client =
            build_blocking_client_https_only_with_timeout(Duration::from_secs(1)).expect("client");
        client
            .get(format!("http://{addr}/"))
            .send()
            .expect("request should succeed");

        let request = worker.join().expect("worker join");
        let lower_request = request.to_ascii_lowercase();
        let expected = format!("user-agent: {}", CLI_USER_AGENT.to_ascii_lowercase());
        assert!(
            lower_request.contains(&expected),
            "expected request headers to include `{expected}`, got: {request}"
        );
    }

    #[test]
    fn update_check_client_does_not_follow_redirects() {
        let mut redirect_server = Server::new();
        let mut target_server = Server::new();
        let target = target_server
            .mock("GET", "/release-data")
            .expect(0)
            .with_status(200)
            .create();
        let redirect = redirect_server
            .mock("GET", "/start")
            .expect(1)
            .with_status(302)
            .with_header("location", &format!("{}/release-data", target_server.url()))
            .create();

        let response = build_update_check_client()
            .expect("client")
            .get(format!("{}/start", redirect_server.url()))
            .send()
            .expect("redirect response should be returned");

        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        redirect.assert();
        target.assert();
    }

    #[test]
    fn update_check_client_does_not_retry_server_errors() {
        let mut server = Server::new();
        let service_error = server
            .mock("GET", "/release-data")
            .expect(1)
            .with_status(503)
            .create();

        let response = build_update_check_client()
            .expect("client")
            .get(format!("{}/release-data", server.url()))
            .send()
            .expect("server error should be returned");

        assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        service_error.assert();
    }

    #[test]
    fn update_check_client_bypasses_configured_system_proxies() {
        let test_binary = env::current_exe().expect("current test binary");
        let output = Command::new(test_binary)
            .args([
                "--ignored",
                "--exact",
                "transport_security::tests::update_check_client_bypasses_configured_system_proxies_in_child",
            ])
            .output()
            .expect("run isolated proxy test");

        assert!(
            output.status.success(),
            "isolated proxy test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "runs only in the isolated proxy-environment child process"]
    fn update_check_client_bypasses_configured_system_proxies_in_child() {
        let mut proxy_server = Server::new();
        let mut origin_server = Server::new();
        let proxy_request = proxy_server
            .mock("GET", mockito::Matcher::Any)
            .expect(1)
            .with_status(200)
            .create();
        let origin_request = origin_server
            .mock("GET", "/release-data")
            .expect(1)
            .with_status(200)
            .create();
        let proxy_url = proxy_server.url();
        let _http_proxy = EnvVarGuard::set("HTTP_PROXY", &proxy_url);
        let _http_proxy_lower = EnvVarGuard::set("http_proxy", &proxy_url);
        let _https_proxy = EnvVarGuard::remove("HTTPS_PROXY");
        let _https_proxy_lower = EnvVarGuard::remove("https_proxy");
        let _all_proxy = EnvVarGuard::remove("ALL_PROXY");
        let _all_proxy_lower = EnvVarGuard::remove("all_proxy");
        let _no_proxy = EnvVarGuard::remove("NO_PROXY");
        let _no_proxy_lower = EnvVarGuard::remove("no_proxy");
        let origin_url = format!("{}/release-data", origin_server.url());

        let system_proxy_response = Client::builder()
            .build()
            .expect("system-proxy client")
            .get(&origin_url)
            .send()
            .expect("system proxy request");
        assert_eq!(system_proxy_response.status(), reqwest::StatusCode::OK);

        let direct_response = build_update_check_client()
            .expect("update-check client")
            .get(&origin_url)
            .send()
            .expect("direct update-check request");
        assert_eq!(direct_response.status(), reqwest::StatusCode::OK);
        proxy_request.assert();
        origin_request.assert();
    }

    #[test]
    fn validate_env_transport_security_rejects_non_https_graphql_url() {
        let env_cfg = EnvConfig {
            graphql_url: "http://example.invalid/graphql".to_string(),
            auth: AuthConfig {
                issuer: "https://issuer.invalid".to_string(),
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        };

        let err = validate_env_transport_security(&env_cfg)
            .expect_err("non-https graphql url should fail");

        assert!(err.to_string().contains("graphql_url must use https"));
    }

    #[test]
    fn validate_env_transport_security_rejects_non_https_auth_issuer() {
        let env_cfg = EnvConfig {
            graphql_url: "https://graphql.invalid".to_string(),
            auth: AuthConfig {
                issuer: "http://issuer.invalid".to_string(),
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        };

        let err = validate_env_transport_security(&env_cfg)
            .expect_err("non-https auth issuer should fail");

        assert!(err.to_string().contains("auth.issuer must use https"));
    }
}
