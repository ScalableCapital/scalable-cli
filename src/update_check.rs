use std::cmp::Ordering;
use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_LENGTH, HeaderMap, LINK};
use serde::Deserialize;
use url::Url;

use crate::transport_security::build_update_check_client;

const RELEASES_API_URL: &str = "https://api.github.com/repos/ScalableCapital/scalable-cli/releases";
const RELEASES_ACCEPT_HEADER: &str = "application/vnd.github+json";
const MAX_PAGES: usize = 5;
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StableVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl StableVersion {
    fn parse_package_version(raw: &str) -> Result<Self> {
        Self::parse_components(raw, "package version")
    }

    fn parse_tag(raw: &str) -> Result<Self> {
        let without_prefix = raw
            .strip_prefix('v')
            .ok_or_else(|| anyhow!("release tag must start with v"))?;
        Self::parse_components(without_prefix, "release tag")
    }

    fn parse_components(raw: &str, subject: &str) -> Result<Self> {
        let mut components = raw.split('.');
        let major = parse_component(components.next(), subject)?;
        let minor = parse_component(components.next(), subject)?;
        let patch = parse_component(components.next(), subject)?;
        if components.next().is_some() {
            bail!("{subject} must have exactly three components");
        }

        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    fn tag(self) -> String {
        format!("v{self}")
    }
}

impl Ord for StableVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl PartialOrd for StableVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for StableVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AvailableUpdate {
    installed: StableVersion,
    available: StableVersion,
}

impl AvailableUpdate {
    pub(crate) fn installed_version(&self) -> String {
        self.installed.to_string()
    }

    pub(crate) fn available_tag(&self) -> String {
        self.available.tag()
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

pub(crate) fn check_for_update_after_login() -> Option<AvailableUpdate> {
    check_for_update_with_lookup(env!("CARGO_PKG_VERSION"), |installed| {
        let client = build_update_check_client()?;
        let endpoint = Url::parse(RELEASES_API_URL)?;
        lookup_update_with_client(&client, &endpoint, installed)
    })
}

fn check_for_update_with_lookup<Lookup>(
    installed_version: &str,
    lookup: Lookup,
) -> Option<AvailableUpdate>
where
    Lookup: FnOnce(StableVersion) -> Result<Option<AvailableUpdate>>,
{
    let installed = StableVersion::parse_package_version(installed_version).ok()?;
    lookup(installed).ok().flatten()
}

fn lookup_update_with_client(
    client: &Client,
    endpoint: &Url,
    installed: StableVersion,
) -> Result<Option<AvailableUpdate>> {
    let deadline = Instant::now() + LOOKUP_TIMEOUT;
    lookup_update_with_client_until(client, endpoint, installed, deadline, Instant::now)
}

fn lookup_update_with_client_until<Now>(
    client: &Client,
    endpoint: &Url,
    installed: StableVersion,
    deadline: Instant,
    mut now: Now,
) -> Result<Option<AvailableUpdate>>
where
    Now: FnMut() -> Instant,
{
    let mut greatest = None;
    let mut page_url = page_url(endpoint, 1)?;

    for page_number in 1..=MAX_PAGES {
        let remaining = deadline
            .checked_duration_since(now())
            .ok_or_else(|| anyhow!("update lookup deadline elapsed"))?;
        if remaining.is_zero() {
            bail!("update lookup deadline elapsed");
        }
        let response = client
            .get(page_url.clone())
            .header(ACCEPT, RELEASES_ACCEPT_HEADER)
            .timeout(remaining)
            .send()
            .context("release lookup request failed")?;
        let next_page = parse_next_page(response.headers(), endpoint, page_number + 1)?;
        let releases = read_releases(response)?;

        for release in releases {
            if release.draft || release.prerelease {
                continue;
            }
            let Ok(version) = StableVersion::parse_tag(&release.tag_name) else {
                continue;
            };
            if greatest.is_none_or(|current| version > current) {
                greatest = Some(version);
            }
        }

        match next_page {
            Some(_) if page_number == MAX_PAGES => {
                bail!("release lookup exceeds page limit");
            }
            Some(next_page) => page_url = next_page,
            None => break,
        }
    }

    Ok(greatest
        .filter(|version| *version > installed)
        .map(|available| AvailableUpdate {
            installed,
            available,
        }))
}

fn page_url(endpoint: &Url, page: usize) -> Result<Url> {
    let mut url = endpoint.clone();
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        bail!("release endpoint must not contain credentials, a query, or a fragment");
    }
    url.set_query(Some(&format!("per_page=100&page={page}")));
    Ok(url)
}

fn read_releases(response: Response) -> Result<Vec<GithubRelease>> {
    if response.status() != StatusCode::OK {
        bail!("release lookup returned HTTP {}", response.status());
    }

    if let Some(content_length) = response.headers().get(CONTENT_LENGTH) {
        let content_length = content_length
            .to_str()
            .context("release lookup content length is not text")?
            .parse::<usize>()
            .context("release lookup content length is invalid")?;
        if content_length > MAX_PAGE_BYTES {
            bail!("release lookup response exceeds size limit");
        }
    }

    let mut body = Vec::with_capacity(MAX_PAGE_BYTES.min(8192));
    response
        .take((MAX_PAGE_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .context("release lookup response read failed")?;
    if body.len() > MAX_PAGE_BYTES {
        bail!("release lookup response exceeds size limit");
    }

    serde_json::from_slice(&body).context("release lookup response is not valid JSON")
}

fn parse_next_page(
    headers: &HeaderMap,
    endpoint: &Url,
    expected_page: usize,
) -> Result<Option<Url>> {
    let mut next_page = None;

    for value in headers.get_all(LINK) {
        let value = value
            .to_str()
            .context("release lookup Link header is not text")?;
        for entry in value.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                bail!("release lookup Link header has an empty entry");
            }
            let (target, parameters) = entry
                .split_once('>')
                .ok_or_else(|| anyhow!("release lookup Link header is malformed"))?;
            let target = target
                .strip_prefix('<')
                .ok_or_else(|| anyhow!("release lookup Link target is malformed"))?;
            if !has_next_relation(parameters) {
                continue;
            }

            let next = Url::parse(target).context("release lookup next Link URL is invalid")?;
            if !is_expected_page(&next, endpoint, expected_page) {
                bail!("release lookup next Link URL is unexpected");
            }
            let next = page_url(endpoint, expected_page)?;
            if next_page.replace(next).is_some() {
                bail!("release lookup has multiple next Link URLs");
            }
        }
    }

    Ok(next_page)
}

fn has_next_relation(parameters: &str) -> bool {
    parameters.split(';').skip(1).any(is_next_relation)
}

fn is_next_relation(parameter: &str) -> bool {
    let Some((name, value)) = parameter.trim().split_once('=') else {
        return false;
    };

    name.eq_ignore_ascii_case("rel")
        && value
            .trim()
            .trim_matches('"')
            .split_ascii_whitespace()
            .any(|relation| relation.eq_ignore_ascii_case("next"))
}

fn is_expected_page(candidate: &Url, endpoint: &Url, expected_page: usize) -> bool {
    if candidate.scheme() != endpoint.scheme()
        || candidate.host_str() != endpoint.host_str()
        || candidate.port_or_known_default() != endpoint.port_or_known_default()
        || !candidate.username().is_empty()
        || candidate.password().is_some()
        || candidate.fragment().is_some()
    {
        return false;
    }

    let expected_page = expected_page.to_string();
    let pairs = candidate.query_pairs().collect::<Vec<_>>();
    pairs.len() == 2
        && pairs
            .iter()
            .any(|(key, value)| key == "per_page" && value == "100")
        && pairs
            .iter()
            .any(|(key, value)| key == "page" && value.as_ref() == expected_page)
}

fn parse_component(component: Option<&str>, subject: &str) -> Result<u64> {
    let component = component.ok_or_else(|| anyhow!("{subject} is missing a component"))?;
    if component.is_empty()
        || !component.bytes().all(|byte| byte.is_ascii_digit())
        || (component.len() > 1 && component.starts_with('0'))
    {
        bail!("{subject} has a non-canonical component");
    }
    component
        .parse::<u64>()
        .with_context(|| format!("{subject} component is out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn endpoint(server: &Server) -> Url {
        Url::parse(&format!(
            "{}/repos/ScalableCapital/scalable-cli/releases",
            server.url()
        ))
        .expect("endpoint URL")
    }

    fn test_client() -> Client {
        build_update_check_client().expect("test update client")
    }

    fn raw_response_endpoint(response: String) -> (Url, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind raw response server");
        let address = listener.local_addr().expect("raw response server address");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept raw response request");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(response.as_bytes())
                .expect("write raw response");
        });
        let endpoint = Url::parse(&format!(
            "http://{address}/repos/ScalableCapital/scalable-cli/releases"
        ))
        .expect("raw response endpoint");

        (endpoint, worker)
    }

    #[test]
    fn parses_only_canonical_stable_versions() {
        assert_eq!(
            StableVersion::parse_package_version("1.2.3").expect("package version"),
            StableVersion {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
        for invalid in ["v1.2.3", "1.2", "1.2.3.4", "01.2.3", "1.2.3-rc.1"] {
            assert!(
                StableVersion::parse_package_version(invalid).is_err(),
                "{invalid}"
            );
        }
        for invalid in ["1.2.3", "v1.2", "v01.2.3", "v1.2.3-rc.1"] {
            assert!(StableVersion::parse_tag(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn recognizes_next_relation_in_link_parameters() {
        for (parameters, expected) in [
            ("; rel=\"next\"", true),
            ("; rel=\"prev next\"", true),
            ("; title=\"next\"", false),
            ("; rel", false),
        ] {
            assert_eq!(has_next_relation(parameters), expected, "{parameters}");
        }
    }

    #[test]
    fn invalid_installed_versions_skip_the_lookup() {
        for invalid in ["1.2.3-rc.1", "01.2.3", "1.2"] {
            assert!(
                check_for_update_with_lookup(invalid, |_| panic!("lookup must not run")).is_none(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn lookup_errors_are_silently_converted_to_no_update() {
        assert!(
            check_for_update_with_lookup("1.0.0", |_| bail!("simulated transport failure"))
                .is_none()
        );
    }

    #[test]
    fn selects_the_highest_valid_stable_release() {
        let mut server = Server::new();
        let response = r#"[
          {"tag_name":"v1.2.0","draft":false,"prerelease":false},
          {"tag_name":"v9.9.9","draft":true,"prerelease":false},
          {"tag_name":"v1.3.0-rc.1","draft":false,"prerelease":true},
          {"tag_name":"not-a-version","draft":false,"prerelease":false},
          {"tag_name":"v1.10.0","draft":false,"prerelease":false}
        ]"#;
        let mock = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .match_header("accept", RELEASES_ACCEPT_HEADER)
            .match_header("user-agent", crate::transport_security::CLI_USER_AGENT)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response)
            .create();

        let update = lookup_update_with_client(
            &test_client(),
            &endpoint(&server),
            StableVersion::parse_package_version("1.1.0").expect("installed version"),
        )
        .expect("lookup should succeed")
        .expect("new update");

        assert_eq!(update.available_tag(), "v1.10.0");
        assert_eq!(update.installed_version(), "1.1.0");
        mock.assert();
    }

    #[test]
    fn ignores_equal_older_and_malformed_stable_candidates() {
        let mut server = Server::new();
        let mock = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .with_status(200)
            .with_body(
                r#"[
                    {"tag_name":"v1.2.3","draft":false,"prerelease":false},
                    {"tag_name":"v1.2.2","draft":false,"prerelease":false},
                    {"tag_name":"v1.2.4-rc.1","draft":false,"prerelease":false},
                    {"tag_name":"v01.2.5","draft":false,"prerelease":false},
                    {"tag_name":"v9.9.9","draft":false,"prerelease":true},
                    {"tag_name":"v9.9.9","draft":true,"prerelease":false}
                ]"#,
            )
            .create();

        let update = lookup_update_with_client(
            &test_client(),
            &endpoint(&server),
            StableVersion::parse_package_version("1.2.3").expect("installed version"),
        )
        .expect("lookup should succeed");

        assert!(update.is_none());
        mock.assert();
    }

    #[test]
    fn rejects_non_success_or_malformed_release_responses() {
        let cases = [(429, "[]"), (200, "not-json")];

        for (status, body) in cases {
            let mut server = Server::new();
            let mock = server
                .mock(
                    "GET",
                    "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
                )
                .with_status(status)
                .with_body(body)
                .create();

            assert!(
                lookup_update_with_client(
                    &test_client(),
                    &endpoint(&server),
                    StableVersion::parse_package_version("1.0.0").expect("installed version"),
                )
                .is_err(),
                "status {status}, body {body}"
            );
            mock.assert();
        }
    }

    #[test]
    fn rejects_an_oversized_chunked_release_response() {
        let mut server = Server::new();
        let oversized_body = vec![b' '; MAX_PAGE_BYTES + 1];
        let mock = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .with_status(200)
            .with_chunked_body(move |writer| writer.write_all(&oversized_body))
            .create();

        assert!(
            lookup_update_with_client(
                &test_client(),
                &endpoint(&server),
                StableVersion::parse_package_version("1.0.0").expect("installed version"),
            )
            .is_err()
        );
        mock.assert();
    }

    #[test]
    fn rejects_declared_oversized_or_invalid_content_lengths() {
        for (content_length, expected_error) in [
            (
                (MAX_PAGE_BYTES + 1).to_string(),
                "release lookup response exceeds size limit",
            ),
            ("not-a-number".to_string(), "invalid content-length parsed"),
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
            );
            let (endpoint, worker) = raw_response_endpoint(response);
            let error = lookup_update_with_client(
                &test_client(),
                &endpoint,
                StableVersion::parse_package_version("1.0.0").expect("installed version"),
            )
            .expect_err("invalid content length must reject the response");

            assert!(
                format!("{error:#}").contains(expected_error),
                "content length {content_length} produced: {error:#}"
            );
            worker.join().expect("raw response server join");
        }
    }

    #[test]
    fn follows_only_a_valid_next_page_link() {
        let mut server = Server::new();
        let endpoint = endpoint(&server);
        let next = page_url(&endpoint, 2).expect("second page URL");
        let first = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_header("link", &format!("<{next}>; rel=\"next\""))
            .with_body(r#"[{"tag_name":"v1.1.0","draft":false,"prerelease":false}]"#)
            .create();
        let second = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=2",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[{"tag_name":"v2.0.0","draft":false,"prerelease":false}]"#)
            .create();

        let update = lookup_update_with_client(
            &test_client(),
            &endpoint,
            StableVersion::parse_package_version("1.0.0").expect("installed version"),
        )
        .expect("lookup should succeed")
        .expect("new update");

        assert_eq!(update.available_tag(), "v2.0.0");
        first.assert();
        second.assert();
    }

    #[test]
    fn follows_githubs_canonical_repository_pagination_link() {
        let mut server = Server::new();
        let endpoint = endpoint(&server);
        let canonical_next = format!(
            "{}/repositories/123456/releases?per_page=100&page=2",
            server.url()
        );
        let first = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .with_status(200)
            .with_header("link", &format!("<{canonical_next}>; rel=\"next\""))
            .with_body(r#"[{"tag_name":"v1.1.0","draft":false,"prerelease":false}]"#)
            .create();
        let second = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=2",
            )
            .with_status(200)
            .with_body(r#"[{"tag_name":"v2.0.0","draft":false,"prerelease":false}]"#)
            .create();

        let update = lookup_update_with_client(
            &test_client(),
            &endpoint,
            StableVersion::parse_package_version("1.0.0").expect("installed version"),
        )
        .expect("lookup should succeed")
        .expect("new update");

        assert_eq!(update.available_tag(), "v2.0.0");
        first.assert();
        second.assert();
    }

    #[test]
    fn rejects_an_unexpected_next_page_link() {
        let mut server = Server::new();
        let mock = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .with_status(200)
            .with_header(
                "link",
                "<https://example.invalid/releases?per_page=100&page=2>; rel=\"next\"",
            )
            .with_body("[]")
            .create();

        assert!(
            lookup_update_with_client(
                &test_client(),
                &endpoint(&server),
                StableVersion::parse_package_version("1.0.0").expect("installed version"),
            )
            .is_err()
        );
        mock.assert();
    }

    #[test]
    fn completes_an_exactly_one_hundred_item_terminal_page() {
        let mut server = Server::new();
        let releases = (0..100)
            .map(|patch| {
                format!(r#"{{"tag_name":"v1.0.{patch}","draft":false,"prerelease":false}}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        let mock = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .with_status(200)
            .with_body(format!("[{releases}]"))
            .create();

        let update = lookup_update_with_client(
            &test_client(),
            &endpoint(&server),
            StableVersion::parse_package_version("1.0.0").expect("installed version"),
        )
        .expect("lookup should succeed")
        .expect("new update");

        assert_eq!(update.available_tag(), "v1.0.99");
        mock.assert();
    }

    #[test]
    fn rejects_next_page_after_the_page_limit() {
        let mut server = Server::new();
        let endpoint = endpoint(&server);
        let mocks = (1..=MAX_PAGES)
            .map(|page| {
                let path = format!(
                    "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page={page}"
                );
                let mut mock = server.mock("GET", path.as_str());
                mock = mock.with_status(200).with_body("[]");
                let next = page_url(&endpoint, page + 1).expect("next page URL");
                mock.with_header("link", &format!("<{next}>; rel=\"next\""))
                    .create()
            })
            .collect::<Vec<_>>();

        assert!(
            lookup_update_with_client(
                &test_client(),
                &endpoint,
                StableVersion::parse_package_version("1.0.0").expect("installed version"),
            )
            .is_err()
        );
        for mock in mocks {
            mock.assert();
        }
    }

    #[test]
    fn completes_a_five_page_scan_when_the_fifth_page_is_terminal() {
        let mut server = Server::new();
        let endpoint = endpoint(&server);
        let mocks = (1..=MAX_PAGES)
            .map(|page| {
                let path = format!(
                    "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page={page}"
                );
                let mut mock = server.mock("GET", path.as_str());
                mock = mock.with_status(200).with_body(format!(
                    r#"[{{"tag_name":"v1.0.{page}","draft":false,"prerelease":false}}]"#
                ));
                if page < MAX_PAGES {
                    let next = page_url(&endpoint, page + 1).expect("next page URL");
                    mock = mock.with_header("link", &format!("<{next}>; rel=\"next\""));
                }
                mock.create()
            })
            .collect::<Vec<_>>();

        let update = lookup_update_with_client(
            &test_client(),
            &endpoint,
            StableVersion::parse_package_version("1.0.0").expect("installed version"),
        )
        .expect("complete scan")
        .expect("newer fifth-page release");

        assert_eq!(update.available_tag(), "v1.0.5");
        for mock in mocks {
            mock.assert();
        }
    }

    #[test]
    fn expires_before_a_later_page_and_discards_earlier_results() {
        let mut server = Server::new();
        let endpoint = endpoint(&server);
        let next = page_url(&endpoint, 2).expect("next page URL");
        let first = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .expect(1)
            .with_status(200)
            .with_header("link", &format!("<{next}>; rel=\"next\""))
            .with_body(r#"[{"tag_name":"v9.0.0","draft":false,"prerelease":false}]"#)
            .create();
        let second = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=2",
            )
            .expect(0)
            .create();
        let start = Instant::now();
        let deadline = start + Duration::from_secs(1);
        let mut calls = 0;

        let result = lookup_update_with_client_until(
            &test_client(),
            &endpoint,
            StableVersion::parse_package_version("1.0.0").expect("installed version"),
            deadline,
            || {
                calls += 1;
                if calls == 1 {
                    start
                } else {
                    deadline + Duration::from_nanos(1)
                }
            },
        );

        assert!(result.is_err());
        first.assert();
        second.assert();
    }

    #[test]
    fn expires_at_the_deadline_before_starting_another_page_request() {
        let mut server = Server::new();
        let endpoint = endpoint(&server);
        let next = page_url(&endpoint, 2).expect("next page URL");
        let first = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=1",
            )
            .expect(1)
            .with_status(200)
            .with_header("link", &format!("<{next}>; rel=\"next\""))
            .with_body(r#"[{"tag_name":"v9.0.0","draft":false,"prerelease":false}]"#)
            .create();
        let second = server
            .mock(
                "GET",
                "/repos/ScalableCapital/scalable-cli/releases?per_page=100&page=2",
            )
            .expect(0)
            .create();
        let start = Instant::now();
        let deadline = start + Duration::from_secs(1);
        let mut calls = 0;

        let error = lookup_update_with_client_until(
            &test_client(),
            &endpoint,
            StableVersion::parse_package_version("1.0.0").expect("installed version"),
            deadline,
            || {
                calls += 1;
                if calls == 1 { start } else { deadline }
            },
        )
        .expect_err("deadline must prevent the next request");

        assert!(format!("{error:#}").contains("deadline elapsed"));
        first.assert();
        second.assert();
    }
}
