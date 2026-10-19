//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
//
// This file is part of meli.
//
// meli is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// meli is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with meli.  If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

//! Connect to a `WebDAV` server.
//!
//! For supported authentication schemes see the [`WebDAVAuthentication`]
//! `enum`.
//!
//! A `WebDAV` connection is used to create a `CardDAV` one, see the
//! [`carddav`](crate::contacts::carddav) module for more
//! details.
//!
//! # Example
//!
//! See [`WebDAVConnection`] and its [`WebDAVConnection::connect`] method.
//!
//! ```rust,no_run
//! use melib::{conf::Secret, utils::webdav::*};
//! # async fn _connect() {
//! let server_conf = WebDAVServerConf {
//!     url: "http://localhost:8081".try_into().unwrap(),
//!     username: Secret::Value(String::new()),
//!     password: Secret::Value(String::new()),
//!     auth: WebDAVAuthentication::None,
//!     danger_accept_invalid_certs: false,
//!     timeout: None,
//! };
//! let mut connection = WebDAVConnection::new(&server_conf).await.unwrap();
//! connection.connect().await.unwrap();
//! # }
//! ```

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures::lock::{
    MappedMutexGuard as FutureMappedMutexGuard, Mutex as FutureMutex,
    MutexGuard as FutureMutexGuard,
};
use indexmap::IndexSet;
use isahc::{
    config::{Configurable as _, RedirectPolicy},
    http::{self, header, status::StatusCode, Request},
    AsyncBody, AsyncReadResponseExt as _, HttpClient,
};
use url::Url;

use crate::{
    conf::Secret,
    email::parser::BytesExt as _,
    error::{Error, ErrorKind, NetworkErrorKind, Result, ResultIntoError as _},
};

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum WebDAVAuthentication {
    Basic,
    BearerToken,
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebDAVServerConf {
    pub url: Url,
    pub username: Secret,
    pub password: Secret,
    pub auth: WebDAVAuthentication,
    #[serde(default)]
    pub danger_accept_invalid_certs: bool,
    #[serde(default)]
    pub timeout: Option<Duration>,
}

/// Whether `url`'s host is a loopback host: the `localhost` domain (RFC 6761
/// semantics — it never leaves the machine), or a loopback IPv4/IPv6 address.
///
/// Same policy as the JMAP `CVE-2024-50624` hardening in
/// [`crate::jmap`]; duplicated here because the `jmap` module is
/// feature-gated independently from `webdav`.
fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Render a URL for an error message without its userinfo section.
///
/// CVE-2026-96869: a URL such as `https://user:password@host/` must never
/// have its embedded credentials echoed into a configuration error or a log
/// line, so error messages print only the scheme, host and port.
fn transport_endpoint(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{}://{host}:{port}", url.scheme()),
        (Some(host), None) => format!("{}://{host}", url.scheme()),
        (None, _) => url.scheme().to_string(),
    }
}

/// Fork hardening (mirrors the JMAP `CVE-2024-50624` TLS-only policy):
/// every WebDAV/CardDAV transport URL must use `https`.
///
/// The connection sends authentication material (HTTP Basic credentials or a
/// Bearer token) with every request, so plaintext `http` is only tolerated
/// for loopback hosts, where the traffic never leaves the machine (local
/// development and test servers). A plaintext connection to a remote host
/// would hand the credentials to any man-in-the-middle on the path.
fn validate_transport_url(url: &Url, what: &str) -> Result<()> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_host(url) => Ok(()),
        scheme => Err(Error::new(format!(
            "Configuration error: {what} must use `https` (plaintext `http` is only allowed \
             for loopback hosts such as 127.0.0.1 or localhost), got scheme `{scheme}` for \
             `{endpoint}`: over a plaintext connection a man-in-the-middle can steal the WebDAV \
             authentication credentials (username/password or bearer token) sent with every \
             request.",
            endpoint = transport_endpoint(url),
        ))
        .set_kind(ErrorKind::Configuration)),
    }?;
    // CVE-2026-96869 (CWE-200): the HTTP engine treats URL userinfo as
    // transport-level Basic credentials, which bypasses meli's credential
    // management and gets echoed by every error / log line that renders the
    // URL. Reject it and never format the userinfo back into the error.
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::new(format!(
            "Configuration error: {what} must not embed credentials in the URL authority \
             section; remove them and configure the WebDAV credentials separately. Embedded \
             URL credentials are never used (CVE-2026-96869). Endpoint: `{endpoint}`.",
            endpoint = transport_endpoint(url),
        ))
        .set_kind(ErrorKind::Configuration));
    }
    Ok(())
}

/// Fork hardening: validate a server-issued redirect target before adopting
/// it as the new endpoint URL.
///
/// Mirrors `JmapConnection::redirect_target`:
///
/// - credentials embedded in the redirect target itself are never honored;
/// - cross-origin targets (different scheme/host/port) are refused, so the
///   connection's authentication material cannot be leaked to another host
///   (a `https` → `http` downgrade is also an origin change and is refused).
fn redirect_target(
    original_url: &Url,
    request_url: &Url,
    response: &http::Response<AsyncBody>,
) -> Result<Option<Url>> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let Some(location) = response.headers().get(header::LOCATION) else {
        return Ok(None);
    };
    let location = String::from_utf8_lossy(location.as_bytes());
    let mut target = match Url::parse(&location) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            request_url.join(&location).map_err(|err| {
                redirect_error(request_url, &location, &err.to_string(), original_url)
            })?
        }
        Err(err) => {
            return Err(redirect_error(
                request_url,
                &location,
                &err.to_string(),
                original_url,
            ))
        }
    };
    // Never honor credentials embedded in the redirect target itself.
    _ = target.set_username("");
    _ = target.set_password(None);
    if target.origin() != original_url.origin() {
        return Err(Error::new(format!(
            "WebDAV request to {request_url} was redirected to {target}, which is of a \
             different origin (scheme/host/port) than {original_url}. Refusing to follow the \
             redirect in order to protect authentication credentials.",
        ))
        .set_kind(ErrorKind::Network(NetworkErrorKind::ProtocolViolation)));
    }
    Ok(Some(target))
}

fn redirect_error(request_url: &Url, location: &str, err: &str, original_url: &Url) -> Error {
    Error::new(format!(
        "Could not resolve redirect location {location:?} of URL {request_url} when \
         connecting to WebDAV server {original_url}: {err}"
    ))
    .set_kind(ErrorKind::Network(NetworkErrorKind::ProtocolViolation))
}

#[derive(Debug)]
pub struct WebDAVConnection {
    client: Arc<HttpClient>,
    pub online_status: OnlineStatus,
    pub server_conf: WebDAVServerConf,
}

/// Metadata of a live `WebDAV` server connection session.
#[derive(Debug)]
pub struct WebDAVSession {
    /// The set of capabilities this `WebDAV` server supports.
    ///
    /// Example value:
    ///
    /// ```text
    /// { "1", "2", "3", "calendar-access", "calendar-auto-scheduling", "addressbook", "extended-mkcol", "add-member", "sync-collection", "quota" }
    /// ```
    pub dav_capabilities: IndexSet<Box<str>>,
    /// HTTP methods this `WebDAV` server supports.
    ///
    /// Example value:
    ///
    /// ```text
    /// { "DELETE", "GET", "HEAD", "MKCALENDAR", "MKCOL", "OPTIONS", "POST", "PROPFIND", "PROPPATCH", "PUT", "REPORT" }
    /// ```
    pub allow_methods: IndexSet<Box<str>>,
    /// The date of connection as provided by the server.
    pub date: Option<Box<str>>,
    /// An identifier provided by the server.
    pub server_id: Option<Box<str>>,
    /// URL path of the authenticated user.
    pub principal: Box<str>,
}

impl WebDAVSession {
    /// Whether the `WebDAV` server replied with the `addressbook` capability in
    /// its `DAV` HTTP headers on response to `OPTIONS` request.
    #[inline]
    pub fn supports_carddav(&self) -> bool {
        self.dav_capabilities.contains("addressbook")
    }
}

#[derive(Clone, Debug)]
#[repr(transparent)]
pub struct OnlineStatus(pub Arc<FutureMutex<(Instant, Result<WebDAVSession>)>>);

impl OnlineStatus {
    /// Returns if session value is `Ok(_)`.
    pub async fn is_ok(&self) -> bool {
        self.0.lock().await.1.is_ok()
    }

    /// Get timestamp of last update.
    pub async fn timestamp(&self) -> Instant {
        self.0.lock().await.0
    }

    /// Get timestamp of last update.
    pub async fn update_timestamp(&self, value: Option<Instant>) {
        self.0.lock().await.0 = value.unwrap_or_else(Instant::now);
    }

    /// Set inner value.
    pub async fn set(
        &self,
        t: Option<Instant>,
        value: Result<WebDAVSession>,
    ) -> Result<WebDAVSession> {
        std::mem::replace(
            &mut (*self.0.lock().await),
            (t.unwrap_or_else(Instant::now), value),
        )
        .1
    }

    pub async fn session_guard(
        &'_ self,
    ) -> Result<FutureMappedMutexGuard<'_, (Instant, Result<WebDAVSession>), WebDAVSession>> {
        let guard = self.0.lock().await;
        if let Err(err) = &guard.1 {
            return Err(err.clone());
        }
        Ok(FutureMutexGuard::map(guard, |status| {
            // SAFETY: we checked if it's an Err() in the previous line, but we cannot do it
            // in here since it's a closure. So unwrap unchecked for API
            // convenience.
            unsafe { status.1.as_mut().unwrap_unchecked() }
        }))
    }
}

impl WebDAVConnection {
    pub async fn new(server_conf: &WebDAVServerConf) -> Result<Self> {
        // Fork hardening: reject plaintext non-loopback URLs before any
        // credentials are configured on the client.
        validate_transport_url(&server_conf.url, "`url`")?;
        let client = HttpClient::builder()
            .dns_cache(isahc::net::dns::DnsCache::Forever)
            .connection_cache_size(8)
            .connection_cache_ttl(Duration::from_secs(30 * 60))
            // Fork policy: only disable the certificate-chain verifier
            // under the user-opted danger flag. The two other isahc TLS
            // relaxation knobs are scoped to the JMAP client by the fork's
            // cve/ audit suite (frozen), so the WebDAV client leaves hostname
            // verification and revocation checks on unconditionally —
            // strictly stronger than upstream, and a smaller blast radius
            // for the danger flag.
            .tls_config(
                isahc::tls::TlsConfig::builder()
                    .danger_accept_invalid_certs(server_conf.danger_accept_invalid_certs)
                    .build(),
            )
            .tcp_nodelay()
            .tcp_keepalive(Duration::new(60 * 9, 0))
            // Fork hardening (same policy as the fork's JMAP client):
            // automatic redirect following must stay disabled because it
            // re-sends credentials to cross-origin redirect targets; the
            // only redirect followed manually (the `/.well-known/carddav`
            // discovery recovery in [`WebDAVConnection::connect`]) validates
            // the target origin first, see [`redirect_target`].
            .redirect_policy(RedirectPolicy::None);
        let client = if let Some(dur) = server_conf.timeout.filter(|dur| *dur != Duration::ZERO) {
            client
                .timeout(dur)
                .connect_timeout(dur + Duration::from_secs(300))
        } else {
            client
        };
        // Resolve Secret -> plaintext only at the moment we need it, right
        // before assembling the auth credentials; never log it.
        let username = server_conf
            .username
            .value_with_timeout(Duration::from_millis(100))
            .await?;
        let password = server_conf
            .password
            .value_with_timeout(Duration::from_millis(100))
            .await?;
        let client = match server_conf.auth {
            WebDAVAuthentication::BearerToken => client
                .authentication(isahc::auth::Authentication::none())
                .default_header(header::AUTHORIZATION, format!("Bearer {password}")),
            WebDAVAuthentication::Basic => client
                .authentication(isahc::auth::Authentication::basic())
                .credentials(isahc::auth::Credentials::new(&username, &password)),
            WebDAVAuthentication::None => client,
        };
        let client = client.build()?;
        let server_conf = server_conf.clone();
        let online_status = OnlineStatus(Arc::new(FutureMutex::new((
            std::time::Instant::now(),
            Err(Error::new("Account is uninitialised.")),
        ))));
        Ok(Self {
            client: Arc::new(client),
            server_conf,
            online_status,
        })
    }

    pub async fn connect(&mut self) -> Result<()> {
        if self.online_status.is_ok().await {
            return Ok(());
        }
        let request = Request::builder()
            .method("OPTIONS")
            .uri(self.server_conf.url.as_str())
            .body(())
            .unwrap();

        let mut resp = match self.client.send_async(request).await.map_err(Error::from) {
            Err(err) => 'block: {
                if matches!(err.kind, ErrorKind::Network(NetworkErrorKind::ProtocolViolation) if self.server_conf.url.scheme() == "http")
                {
                    // attempt recovery by trying https://
                    self.server_conf.url.set_scheme("https").expect(
                        "set_scheme to https must succeed here because we checked earlier that \
                         current scheme is http",
                    );
                    let request = Request::builder()
                        .method("OPTIONS")
                        .uri(self.server_conf.url.as_str())
                        .body(())
                        .unwrap();
                    if let Ok(s) = self.client.send_async(request).await {
                        tracing::error!(
                            "WebDAV server URL should start with `https`. Please correct your \
                             configuration value. Its current value is `{}`.",
                            self.server_conf.url
                        );
                        break 'block s;
                    }
                }

                let err = Error::new(format!(
                    "Could not connect to WebDAV server endpoint for {}.\nError connecting to \
                     server: {}",
                    self.server_conf.url, err
                ))
                .set_source(Some(Box::new(err)));
                _ = self.online_status.set(None, Err(err.clone())).await;
                return Err(err);
            }
            Ok(s) => s,
        };
        let req_instant = Instant::now();

        if !resp.status().is_success() {
            let kind: crate::error::NetworkErrorKind = resp.status().into();
            let res_text = resp.text().await.unwrap_or_default();
            let mut err = Error::new(format!(
                "Could not connect to WebDAV server endpoint for {}. Reply from server: {}",
                self.server_conf.url, res_text
            ))
            .set_kind(kind.into());

            let mut retry = false;
            if resp.status() == 404 && self.server_conf.url.path() == "/" {
                // attempt recovery by trying "/.well-known/carddav"
                let mut well_known = self.server_conf.url.clone();
                well_known.set_path("/.well-known/carddav");
                let request = Request::builder()
                    .method("PROPFIND")
                    .uri(well_known.as_str())
                    .body(())
                    .unwrap();
                if let Ok(s) = self.client.send_async(request).await {
                    // Fork hardening: only follow the redirect if the target
                    // passes the origin/scheme validation, see
                    // [`redirect_target`].
                    if let Ok(Some(redirect)) =
                        redirect_target(&self.server_conf.url.clone(), &well_known, &s)
                    {
                        self.server_conf.url = redirect;
                        let request = Request::builder()
                            .method("OPTIONS")
                            .uri(self.server_conf.url.as_str())
                            .body(())
                            .unwrap();
                        if let Ok(s) = self.client.send_async(request).await {
                            resp = s;
                            retry = true;
                        }
                    }
                }
            } else if resp.status() == 401 {
                let mut supports_bearer = false;
                let mut supports_basic = false;
                for val in resp.headers().get_all(header::WWW_AUTHENTICATE).iter() {
                    supports_bearer |= val.as_bytes().contains_subsequence(b"Bearer".as_slice());
                    supports_basic |= val.as_bytes().contains_subsequence(b"Basic".as_slice());
                }
                match (self.server_conf.auth, supports_bearer, supports_basic) {
                    (WebDAVAuthentication::Basic, true, _) => {
                        err = err
                            .set_details(
                                "The server rejected your authentication credentials because it \
                                 expects authentication with a Bearer token instead of a \
                                 password. Check your provider's client connection documentation. \
                                 Note that to use Bearer token authentication, you must \
                                 explicitly set `use_token=true` in the account's configuration.",
                            )
                            .set_kind(ErrorKind::Authentication);
                    }
                    (WebDAVAuthentication::BearerToken, false, true) => {
                        err = err
                            .set_details(
                                "The server rejected your authentication credentials because it \
                                 expects authentication with a username and password but \
                                 `use_token` is set to `true`. Try setting it to `false`.",
                            )
                            .set_kind(ErrorKind::Authentication);
                    }
                    (auth_method, false, false) => {
                        let schemes = resp
                            .headers()
                            .get_all(header::WWW_AUTHENTICATE)
                            .iter()
                            .map(|val| String::from_utf8_lossy(val.as_bytes()))
                            .collect::<Vec<_>>();
                        if matches!(auth_method, WebDAVAuthentication::None) {
                            err = err
                                .set_details("The server requires authentication.")
                                .set_kind(ErrorKind::Authentication);
                        }
                        if schemes.is_empty() {
                            err = err
                                .set_details(
                                    "The server fails to report what authentication schemes it \
                                     supports. Please report this to your e-mail provider! (The \
                                     server is expected to provide the supported schemes with the \
                                     WWW-Authenticate HTTP header)",
                                )
                                .set_kind(ErrorKind::ProtocolError);
                        } else {
                            err = err
                                .set_details(format!(
                                    "The server does not support any of the implemented \
                                     authentication mechanisms (Basic or Bearer token). Here are \
                                     the authentication schemes it reports to support: {}",
                                    schemes.join(", ")
                                ))
                                .set_kind(ErrorKind::Authentication);
                        }
                    }
                    (WebDAVAuthentication::BearerToken, true, _)
                    | (WebDAVAuthentication::Basic, _, true) => {
                        err = err
                            .set_details(
                                "The server rejected your authentication credentials. Confirm you \
                                 are not using an invalid password or token value.",
                            )
                            .set_kind(ErrorKind::Authentication);
                    }
                    (WebDAVAuthentication::None, supports_basic, supports_bearer) => {
                        err = err
                            .set_details("The server requires authentication.")
                            .set_kind(ErrorKind::Authentication);
                        // Either basic or bearer token is support here, because the `false, false`
                        // pattern was caught in another match guard.
                        if supports_basic {
                            err = err.set_details(
                                "It supports HTTP Basic authentication (username and password).",
                            );
                        }
                        if supports_bearer {
                            err = err.set_details(
                                "It also supports authentication using a Bearer Token.",
                            );
                        }
                    }
                }
            }
            if !retry {
                _ = self
                    .online_status
                    .set(Some(req_instant), Err(err.clone()))
                    .await;
                return Err(err);
            }
        }

        let _res_text = match resp.text().await {
            Err(err) => {
                let err = Error::new(format!(
                    "Could not connect to WebDAV server endpoint for {}\nReply from server: {}",
                    self.server_conf.url, err
                ))
                .set_source(Some(Box::new(err)));
                _ = self
                    .online_status
                    .set(Some(req_instant), Err(err.clone()))
                    .await;
                return Err(err);
            }
            Ok(s) => s,
        };
        if resp.headers().get("dav").is_none() {
            let err = Error::new(format!(
                "WebDAV server {} did not reply to OPTIONS request with a `DAV` capability \
                 header.\nHeaders in server reply:\n{:#?}",
                self.server_conf.url,
                resp.headers()
            ))
            .set_kind(ErrorKind::ProtocolNotSupported);
            _ = self
                .online_status
                .set(Some(req_instant), Err(err.clone()))
                .await;
            return Err(err);
        };
        let allow_methods: IndexSet<Box<str>> = resp
            .headers()
            .get_all("allow")
            .iter()
            .map(|s| s.to_str().unwrap())
            .flat_map(|s| s.split(", "))
            .map(|s| s.into())
            .collect();
        let dav_capabilities = resp
            .headers()
            .get_all("dav")
            .iter()
            .map(|s| s.to_str().unwrap())
            .flat_map(|s| s.split(", "))
            .map(|s| s.into())
            .collect();
        if !allow_methods.contains("PROPFIND") {
            return Err(Error::new(format!(
                "WebDAV connection failed: the server does not support the `PROPFIND` HTTP \
                 method. The server replied with the following allowed methods in the `allow` \
                 http header: {:?}",
                allow_methods
            ))
            .set_kind(ErrorKind::ProtocolNotSupported));
        }
        let request = Request::builder()
            .method("PROPFIND")
            .uri(self.server_conf.url.as_str())
            .header(header::CONTENT_TYPE, "application/xml; content-type=utf-8")
            .header("Depth", "0")
            .body(
                r#"<d:propfind xmlns:d="DAV:">
  <d:prop>
     <d:current-user-principal />
  </d:prop>
</d:propfind>"#,
            )
            .unwrap();
        let mut resp = self.client.send_async(request).await?;
        if !resp.status().is_success() {
            let kind: crate::error::NetworkErrorKind = resp.status().into();
            let res_text = resp.text().await.unwrap_or_default();
            let err = Error::new(format!(
                "Could not connect to WebDAV server endpoint for {}. Reply from server: {}",
                self.server_conf.url, res_text
            ))
            .set_kind(kind.into());

            _ = self
                .online_status
                .set(Some(req_instant), Err(err.clone()))
                .await;
            return Err(err);
        }

        tracing::trace!("propfind current user principal {:?}", resp);
        let text = resp.text().await?;
        let mut principals = vec![];
        {
            let multistatus: Multistatus<CurrentUserPrincipalProp> = quick_xml::de::from_str(&text)
                .chain_err_summary(|| "Could not deserialize PROPFIND response from XML")?;
            for response in multistatus.response {
                for propstat in response.propstat {
                    if let (data, Some(StatusCode::OK)) = (
                        propstat.prop.current_user_principal.href,
                        StatusCode::from_status_line(&propstat.status),
                    ) {
                        if !data.is_empty() {
                            principals.push(data);
                        }
                    }
                }
            }
        }
        if principals.is_empty() {
            return Err(Error::new(format!(
                "Could not connect to WebDAV server endpoint for {}",
                self.server_conf.url
            ))
            .set_details(format!(
                "Server returned no principals in PROPFIND response: {text:?}"
            ))
            .set_kind(ErrorKind::ProtocolError));
        }
        let principal = principals.remove(0);

        let session = WebDAVSession {
            allow_methods,
            dav_capabilities,
            date: resp
                .headers()
                .get("date")
                .and_then(|d| d.to_str().ok())
                .map(Into::into),
            server_id: resp
                .headers()
                .get("server")
                .and_then(|s| s.to_str().ok())
                .map(Into::into),
            principal,
        };
        _ = self
            .online_status
            .set(Some(Instant::now()), Ok(session))
            .await;

        Ok(())
    }

    pub async fn send_async<B>(&self, request: Request<B>) -> Result<http::Response<AsyncBody>>
    where
        B: Into<AsyncBody>,
    {
        let req_instant = Instant::now();
        let mut resp = self.client.send_async(request).await?;
        if !resp.status().is_success() {
            let kind: crate::error::NetworkErrorKind = resp.status().into();
            let res_text = resp.text().await.unwrap_or_default();
            let err = Error::new(format!(
                "Could not connect to WebDAV server endpoint for {}. Reply from server: {}",
                self.server_conf.url, res_text
            ))
            .set_kind(kind.into());

            _ = self
                .online_status
                .set(Some(req_instant), Err(err.clone()))
                .await;
            return Err(err);
        }
        self.online_status.update_timestamp(Some(req_instant)).await;
        Ok(resp)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Multistatus<Prop: std::fmt::Debug> {
    #[serde(rename = "response")]
    pub response: Vec<Response<Prop>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response<Prop: std::fmt::Debug> {
    #[serde(rename = "href")]
    pub href: Box<str>,
    #[serde(rename = "propstat")]
    pub propstat: Vec<Propstat<Prop>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Propstat<Prop: std::fmt::Debug> {
    #[serde(rename = "status")]
    pub status: Box<str>,
    #[serde(rename = "prop")]
    pub prop: Prop,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CurrentUserPrincipalProp {
    #[serde(rename = "current-user-principal")]
    pub current_user_principal: HrefValue,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HrefValue {
    #[serde(rename = "href")]
    pub href: Box<str>,
}

mod private {
    pub trait Sealed {}
}

/// Parse an HTTP protocol `Status-Line` grammar item into
/// `http::status::StatusCode`.
///
/// ```rust
/// use isahc::http::status::StatusCode;
/// use melib::utils::webdav::FromStatusLine;
///
/// assert_eq!(
///     StatusCode::from_status_line("HTTP/1.1 200 OK").unwrap(),
///     StatusCode::OK
/// );
/// assert_eq!(
///     StatusCode::from_status_line("HTTP/1.1 404 Not Found").unwrap(),
///     StatusCode::NOT_FOUND
/// );
/// ```
pub trait FromStatusLine: private::Sealed + Sized {
    /// Parse a `Status-Line` into `http::status::StatusCode`.
    ///
    /// `Status-Line` is defined in [RFC2068] [Section 6.1 "Status-Line"] as:
    ///
    /// ```text
    /// Status-Line = HTTP-Version SP Status-Code SP Reason-Phrase CRLF
    /// ```
    ///
    /// Reason phrase is ignored, because "The client is not required to examine
    /// or display the Reason-Phrase".
    ///
    /// [RFC2068]: https://www.rfc-editor.org/rfc/rfc2068
    /// [Section 6.1 "Status-Line"]: https://www.rfc-editor.org/rfc/rfc2068#section-6.1
    fn from_status_line(_: &str) -> Option<Self>;
}

impl private::Sealed for StatusCode {}

impl FromStatusLine for StatusCode {
    fn from_status_line(mut val: &str) -> Option<Self> {
        // Example value: "HTTP/1.1 200 OK"
        if !val.is_ascii() {
            return None;
        }
        if val.ends_with("\r\n") {
            val = val.trim_end_matches("\r\n");
        }
        let mut split_iter = val.split(' ');
        _ = split_iter.next()?;
        let status_code_s = split_iter.next()?;
        // Ignore Reason-Phrase
        // _ = split_iter.next()?;
        Self::from_bytes(status_code_s.as_bytes()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_status_line() {
        assert_eq!(
            StatusCode::from_status_line("HTTP/1.1 200 OK").unwrap(),
            StatusCode::OK
        );
        assert_eq!(
            StatusCode::from_status_line("HTTP/1.1 404 Not Found").unwrap(),
            StatusCode::NOT_FOUND
        );
        // Trailing CRLF is tolerated.
        assert_eq!(
            StatusCode::from_status_line("HTTP/1.1 207 Multi-Status\r\n").unwrap(),
            StatusCode::MULTI_STATUS
        );
        // Non-ASCII or malformed lines are rejected.
        assert!(StatusCode::from_status_line("HTTP/1.1").is_none());
        assert!(StatusCode::from_status_line("garbage").is_none());
        assert!(StatusCode::from_status_line("HTTP/1.1 δ.getDay").is_none());
    }

    /// Fork hardening regression tests: credentials must never be sent over
    /// plaintext `http` to non-loopback hosts.
    #[test]
    fn test_transport_url_policy() {
        for url in ["https://dav.example.com", "https://dav.example.com:8443/"] {
            validate_transport_url(&Url::parse(url).unwrap(), "`url`").unwrap();
        }
        for url in [
            "http://localhost:8081",
            "http://127.0.0.1",
            "http://[::1]:8080/",
            "http://LOCALHOST/dav",
        ] {
            validate_transport_url(&Url::parse(url).unwrap(), "`url`").unwrap();
        }
        for url in [
            "http://dav.example.com",
            "http://dav.example.com:8080/",
            "ftp://dav.example.com",
        ] {
            let err = validate_transport_url(&Url::parse(url).unwrap(), "`url`").unwrap_err();
            assert_eq!(err.kind, ErrorKind::Configuration, "for url {url}: {err}");
        }
    }

    /// Fork hardening regression tests: server-issued redirects are only
    /// followed when they stay same-origin and do not carry credentials.
    fn redirect_response(status: u16, location: Option<&str>) -> http::Response<AsyncBody> {
        let mut builder = http::Response::builder().status(status);
        if let Some(location) = location {
            builder = builder.header(header::LOCATION, location);
        }
        builder.body(AsyncBody::from(())).unwrap()
    }

    #[test]
    fn test_redirect_target_same_origin() {
        let original = Url::parse("https://dav.example.com/").unwrap();
        let request = Url::parse("https://dav.example.com/.well-known/carddav").unwrap();
        // Absolute and relative same-origin targets are followed.
        assert_eq!(
            redirect_target(
                &original,
                &request,
                &redirect_response(301, Some("https://dav.example.com/dav/user/"))
            )
            .unwrap()
            .unwrap(),
            Url::parse("https://dav.example.com/dav/user/").unwrap()
        );
        assert_eq!(
            redirect_target(
                &original,
                &request,
                &redirect_response(302, Some("/dav/user/"))
            )
            .unwrap()
            .unwrap(),
            Url::parse("https://dav.example.com/dav/user/").unwrap()
        );
        // Credentials embedded in the redirect target are stripped.
        assert_eq!(
            redirect_target(
                &original,
                &request,
                &redirect_response(301, Some("https://user:pass@dav.example.com/dav/user/"))
            )
            .unwrap()
            .unwrap(),
            Url::parse("https://dav.example.com/dav/user/").unwrap()
        );
        // Non-redirect statuses are ignored.
        assert!(
            redirect_target(&original, &request, &redirect_response(404, None))
                .unwrap()
                .is_none()
        );
        assert!(redirect_target(
            &original,
            &request,
            &redirect_response(200, Some("/dav/user/"))
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn test_redirect_target_cross_origin_rejected() {
        let original = Url::parse("https://dav.example.com/").unwrap();
        let request = Url::parse("https://dav.example.com/.well-known/carddav").unwrap();
        // Different host.
        let err = redirect_target(
            &original,
            &request,
            &redirect_response(301, Some("https://evil.example/dav/")),
        )
        .unwrap_err();
        assert!(matches!(
            err.kind,
            ErrorKind::Network(NetworkErrorKind::ProtocolViolation)
        ));
        // https -> http downgrade on the same host is an origin change too.
        let err = redirect_target(
            &original,
            &request,
            &redirect_response(302, Some("http://dav.example.com/dav/")),
        )
        .unwrap_err();
        assert!(matches!(
            err.kind,
            ErrorKind::Network(NetworkErrorKind::ProtocolViolation)
        ));
        // Different port.
        redirect_target(
            &original,
            &request,
            &redirect_response(301, Some("https://dav.example.com:8443/dav/")),
        )
        .unwrap_err();
    }

    /// `WebDAV` `PROPFIND` multistatus XML parsing, offline (no network).
    #[test]
    fn test_current_user_principal_propfind_deserialization() {
        let xml = r#"<?xml version="1.0" encoding="utf-8" ?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/.well-known/carddav</D:href>
    <D:propstat>
      <D:prop>
        <D:current-user-principal>
          <D:href>/dav/user@example.com/</D:href>
        </D:current-user-principal>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let multistatus: Multistatus<CurrentUserPrincipalProp> =
            quick_xml::de::from_str(xml).unwrap();
        assert_eq!(multistatus.response.len(), 1);
        let mut principals = vec![];
        for response in multistatus.response {
            for propstat in response.propstat {
                if let (data, Some(StatusCode::OK)) = (
                    propstat.prop.current_user_principal.href,
                    StatusCode::from_status_line(&propstat.status),
                ) {
                    if !data.is_empty() {
                        principals.push(data);
                    }
                }
            }
        }
        assert_eq!(principals, vec![Box::from("/dav/user@example.com/")]);
    }

    /// `CardDAV` `addressbook-home-set` `PROPFIND` multistatus XML parsing,
    /// offline (no network).
    #[test]
    fn test_addressbook_home_set_propfind_deserialization() {
        let xml = r#"<?xml version="1.0" encoding="utf-8" ?>
<D:multistatus xmlns:D="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <D:response>
    <D:href>/dav/user@example.com/</D:href>
    <D:propstat>
      <D:prop>
        <card:addressbook-home-set>
          <D:href>/dav/user@example.com/addressbooks/</D:href>
        </card:addressbook-home-set>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let multistatus: Multistatus<crate::contacts::carddav::AddressBookHomeSetProp> =
            quick_xml::de::from_str(xml).unwrap();
        assert_eq!(multistatus.response.len(), 1);
        let propstat = &multistatus.response[0].propstat[0];
        assert_eq!(
            propstat.prop.addressbook_home_set.href.as_ref(),
            "/dav/user@example.com/addressbooks/"
        );
        assert_eq!(
            StatusCode::from_status_line(&propstat.status),
            Some(StatusCode::OK)
        );
    }
}
