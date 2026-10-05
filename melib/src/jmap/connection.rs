/*
 * meli - jmap module.
 *
 * Copyright 2019 Manos Pitsidianakis
 * Copyright 2026 Kyle Lee
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

use std::{
    convert::TryFrom,
    sync::Arc,
    time::{Duration, Instant},
};

use futures::lock::{MappedMutexGuard as FutureMappedMutexGuard, Mutex as FutureMutex};
use isahc::{
    config::{Configurable, RedirectPolicy},
    http, AsyncReadResponseExt, HttpClient,
};
use url::Url;

use crate::{
    email::parser::BytesExt,
    error::{Error, ErrorKind, NetworkErrorKind, Result},
    jmap::{
        argument::Argument,
        capabilities::*,
        deserialize_from_str,
        email::{EmailChanges, EmailGet, EmailObject},
        identity::{Identity, IdentityGet, IdentitySet},
        methods::{Changes, ChangesResponse, Get, GetResponse, MethodResponse, ResultField, Set},
        objects::{Id, State},
        protocol::{self, JmapMailCapability, Request},
        session::Session,
        validate_session_urls, validate_transport_url, JmapServerConf, Store,
    },
    BackendEvent, Flag, RefreshEvent, RefreshEventKind,
};

#[derive(Debug)]
pub struct JmapConnection {
    pub request_no: Arc<FutureMutex<usize>>,
    pub client: Arc<HttpClient>,
    pub server_conf: JmapServerConf,
    pub store: Arc<Store>,
}

/// Maximum number of redirects followed manually per request before giving
/// up with an error.
const MAX_REDIRECTS: usize = 5;

/// HTTP status codes whose responses are treated as redirects.
fn is_redirect_status(status: http::StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

impl JmapConnection {
    pub fn new(server_conf: &JmapServerConf, store: Arc<Store>) -> Result<Self> {
        let client = HttpClient::builder()
            .dns_cache(isahc::net::dns::DnsCache::Forever)
            .connection_cache_size(8)
            .connection_cache_ttl(Duration::from_secs(30 * 60))
            .default_header(http::header::CONTENT_TYPE, "application/json")
            .tls_config(
                isahc::tls::TlsConfig::builder()
                    .danger_accept_invalid_certs(server_conf.danger_accept_invalid_certs)
                    .danger_accept_invalid_hosts(server_conf.danger_accept_invalid_certs)
                    .danger_accept_revoked_certs(server_conf.danger_accept_invalid_certs)
                    .build(),
            )
            .tcp_nodelay()
            .tcp_keepalive(Duration::new(60 * 9, 0))
            // Redirects are followed manually, see [`MAX_REDIRECTS`]. isahc
            // automatic redirect following must stay disabled: it re-sends
            // credentials to cross-origin redirect targets (curl engine auth
            // via CURLOPT_HTTPAUTH set by `.authentication()`, and default
            // headers re-attached by isahc's `DefaultHeaders` interceptor),
            // see [`JmapConnection::redirect_target`].
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
        let password_value = server_conf.server_password.value()?;
        let client = if server_conf.use_token {
            client
                .authentication(isahc::auth::Authentication::none())
                .default_header(
                    http::header::AUTHORIZATION,
                    format!("Bearer {}", password_value),
                )
        } else {
            client
                .authentication(isahc::auth::Authentication::basic())
                .credentials(isahc::auth::Credentials::new(
                    &server_conf.server_username,
                    &password_value,
                ))
        };
        let client = client.build()?;
        let server_conf = server_conf.clone();
        Ok(Self {
            request_no: Arc::new(FutureMutex::new(0)),
            client: Arc::new(client),
            server_conf,
            store,
        })
    }

    pub async fn connect(&mut self) -> Result<()> {
        if self.store.online_status.is_ok().await {
            return Ok(());
        }

        if let Err(err) = validate_transport_url(&self.server_conf.server_url, "`server_url`") {
            _ = self.store.online_status.set(None, Err(err.clone())).await;
            return Err(err);
        }

        fn to_well_known(uri: &Url) -> Url {
            let mut uri = uri.clone();
            uri.set_path(".well-known/jmap");
            uri
        }

        let mut jmap_session_resource_url = to_well_known(&self.server_conf.server_url);

        let mut resp = match self.get_async(&jmap_session_resource_url).await {
            Err(err) => 'block: {
                if matches!(err.kind, ErrorKind::Network(NetworkErrorKind::ProtocolViolation) if self.server_conf.server_url.scheme() == "http")
                {
                    // attempt recovery by trying https://
                    self.server_conf.server_url.set_scheme("https").expect(
                        "set_scheme to https must succeed here because we checked earlier that \
                         current scheme is http",
                    );
                    jmap_session_resource_url = to_well_known(&self.server_conf.server_url);
                    if let Ok(s) = self.get_async(&jmap_session_resource_url).await {
                        tracing::error!(
                            "Account {} server URL should start with `https`. Please correct your \
                             configuration value. Its current value is `{}`.",
                            self.store.account_name,
                            self.server_conf.server_url
                        );
                        break 'block s;
                    }
                }

                let err = Error::new(format!(
                    "Could not connect to JMAP server endpoint for {}. Is your server url setting \
                     correct? (i.e. \"jmap.mailserver.org\") (Note: only session resource \
                     discovery via /.well-known/jmap is supported. DNS SRV records are not \
                     supported)\n\nError connecting to server: {err}",
                    self.server_conf.server_url
                ))
                .set_source(Some(Arc::new(err)));
                _ = self.store.online_status.set(None, Err(err.clone())).await;
                return Err(err);
            }
            Ok(s) => s,
        };
        let req_instant = Instant::now();

        if !resp.status().is_success() {
            let kind: crate::error::NetworkErrorKind = resp.status().into();
            let res_text = resp.text().await.unwrap_or_default();
            let mut err = Error::new(format!(
                "Could not connect to JMAP server endpoint for {}. Reply from server: {res_text}",
                self.server_conf.server_url
            ))
            .set_kind(kind.into());
            if resp.status() == 401 {
                let mut supports_bearer = false;
                let mut supports_basic = false;
                for val in resp
                    .headers()
                    .get_all(http::header::WWW_AUTHENTICATE)
                    .iter()
                {
                    supports_bearer |= val.as_bytes().contains_subsequence(b"Bearer".as_slice());
                    supports_basic |= val.as_bytes().contains_subsequence(b"Basic".as_slice());
                }
                match (self.server_conf.use_token, supports_bearer, supports_basic) {
                    (false, true, _) => {
                        err = err.set_details(
                            "The server rejected your authentication credentials because it \
                             expects authentication with a Bearer token instead of a password. \
                             Check your provider's client connection documentation. Note that to \
                             use Bearer token authentication, you must explicitly set \
                             `use_token=true` in the account's configuration.",
                        );
                    }
                    (true, false, true) => {
                        err = err.set_details(
                            "The server rejected your authentication credentials because it \
                             expects authentication with a username and password but `use_token` \
                             is set to `true`. Try setting it to `false`.",
                        );
                    }
                    (_, false, false) => {
                        let schemes = resp
                            .headers()
                            .get_all(http::header::WWW_AUTHENTICATE)
                            .iter()
                            .map(|val| String::from_utf8_lossy(val.as_bytes()).to_string())
                            .collect::<Vec<String>>();
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
                            err = err.set_details(format!(
                                "The server does not support any of the implemented \
                                 authentication mechanisms (Basic or Bearer token). Here are the \
                                 authentication schemes it reports to support: {}",
                                schemes.join(", ")
                            ));
                        }
                    }
                    (true, true, _) | (false, _, true) => {
                        err = err.set_details(
                            "The server rejected your authentication credentials. Confirm you are \
                             not using an invalid password or token value.",
                        );
                    }
                }
            }
            _ = self
                .store
                .online_status
                .set(Some(req_instant), Err(err.clone()))
                .await;
            return Err(err);
        }

        let res_text = match resp.text().await {
            Err(err) => {
                let err = Error::new(format!(
                    "Could not connect to JMAP server endpoint for {}. Is your server url setting \
                     correct? (i.e. \"jmap.mailserver.org\") (Note: only session resource \
                     discovery via /.well-known/jmap is supported. DNS SRV records are not \
                     supported)\n\nReply from server: {err}",
                    self.server_conf.server_url
                ))
                .set_source(Some(Arc::new(err)));
                _ = self
                    .store
                    .online_status
                    .set(Some(req_instant), Err(err.clone()))
                    .await;
                return Err(err);
            }
            Ok(s) => s,
        };

        let session: Session = match deserialize_from_str(&res_text) {
            Err(err) => {
                let err = Error::new(format!(
                    "Could not connect to JMAP server endpoint for {}. Is your server url setting \
                     correct? (i.e. \"jmap.mailserver.org\") (Note: only session resource \
                     discovery via /.well-known/jmap is supported. DNS SRV records are not \
                     supported)\n\nReply from server: {res_text}",
                    self.server_conf.server_url
                ))
                .set_source(Some(Arc::new(err)));
                _ = self
                    .store
                    .online_status
                    .set(Some(req_instant), Err(err.clone()))
                    .await;
                return Err(err);
            }
            Ok(s) => s,
        };
        if let Err(err) = validate_session_urls(&session) {
            _ = self
                .store
                .online_status
                .set(Some(req_instant), Err(err.clone()))
                .await;
            return Err(err);
        }
        macro_rules! check_for_cap {
            ($cap:ident) => {{
                if !session.capabilities.contains_key($cap::URI) {
                    let err = Error::new(format!(
                        "Server {} did not return {name} ({uri}). Returned capabilities were: {}",
                        self.server_conf.server_url,
                        session
                            .capabilities
                            .keys()
                            .map(String::as_str)
                            .collect::<Vec<&str>>()
                            .join(", "),
                        name = $cap::NAME,
                        uri = $cap::URI
                    ));
                    _ = self
                        .store
                        .online_status
                        .set(Some(req_instant), Err(err.clone()))
                        .await;
                    return Err(err);
                }
            }};
        }

        check_for_cap! { JmapCoreCapability };
        check_for_cap! { JmapMailCapability };

        self.store
            .core_capabilities
            .lock()
            .unwrap()
            .clone_from(&session.capabilities);
        let mail_account_id = session.mail_account_id()?;
        {
            let mut metadata = self.store.metadata.lock().unwrap();
            metadata.insert("session".into(), serde_json::json! {session});
        };
        _ = self
            .store
            .online_status
            .set(Some(req_instant), Ok(session))
            .await;

        // Fetch account identities.

        let mut id_list = {
            let mut req = Request::new(self.request_no.clone());
            let identity_get = IdentityGet::new(Get::new().account_id(mail_account_id.clone()));
            req.add_call(&identity_get).await;
            let res_text = self
                .post_async(None, serde_json::to_string(&req)?)
                .await?
                .text()
                .await?;
            let mut v: MethodResponse = match deserialize_from_str(&res_text) {
                Err(err) => {
                    _ = self
                        .store
                        .online_status
                        .set(Some(req_instant), Err(err.clone()))
                        .await;
                    return Err(err);
                }
                Ok(s) => s,
            };
            let GetResponse::<Identity> { list, .. } =
                GetResponse::<Identity>::try_from(v.take_first()?)?;
            list
        };
        if id_list.is_empty() {
            let mut req = Request::new(self.request_no.clone());
            let identity_set = IdentitySet(
                Set::<Identity>::new(None)
                    .account_id(mail_account_id.clone())
                    .create(Some({
                        let address =
                            crate::email::Address::try_from(self.store.main_identity.as_str())
                                .unwrap_or_else(|_| {
                                    crate::email::Address::new(
                                        None::<&str>,
                                        self.store.main_identity.clone(),
                                    )
                                });
                        let id: Id<Identity> = Id::new_random();
                        tracing::trace!(
                            "identity id = {}, {:#?}",
                            id,
                            Identity {
                                id: id.clone(),
                                name: address.get_display_name().unwrap_or_default().into(),
                                email: address.get_email().into(),
                                ..Identity::default()
                            }
                        );
                        indexmap! {
                            id.clone().into() => Identity {
                                id,
                                name: address.get_display_name().unwrap_or_default().into(),
                                email: address.get_email().into(),
                                ..Identity::default()
                            }
                        }
                    })),
            );
            req.add_call(&identity_set).await;
            let res_text = self
                .post_async(None, serde_json::to_string(&req)?)
                .await?
                .text()
                .await?;
            let _: MethodResponse = match deserialize_from_str(&res_text) {
                Err(err) => {
                    _ = self
                        .store
                        .online_status
                        .set(Some(req_instant), Err(err.clone()))
                        .await;
                    return Err(err);
                }
                Ok(s) => s,
            };
            let mut req = Request::new(self.request_no.clone());
            let identity_get = IdentityGet::new(Get::new().account_id(mail_account_id.clone()));
            req.add_call(&identity_get).await;
            let res_text = self
                .post_async(None, serde_json::to_string(&req)?)
                .await?
                .text()
                .await?;
            let mut v: MethodResponse = match deserialize_from_str(&res_text) {
                Err(err) => {
                    _ = self
                        .store
                        .online_status
                        .set(Some(req_instant), Err(err.clone()))
                        .await;
                    return Err(err);
                }
                Ok(s) => s,
            };
            let GetResponse::<Identity> { list, .. } =
                GetResponse::<Identity>::try_from(v.take_first()?)?;
            id_list = list;
        }
        self.session_guard().await?.identities =
            id_list.into_iter().map(|id| (id.id.clone(), id)).collect();

        Ok(())
    }

    #[inline]
    pub async fn session_guard(
        &'_ self,
    ) -> Result<FutureMappedMutexGuard<'_, (Instant, Result<Session>), Session>> {
        self.store.online_status.session_guard().await
    }

    #[inline]
    pub fn add_backend_event(&self, ev: BackendEvent) {
        (self.store.event_consumer)(self.store.account_hash, ev);
    }

    pub async fn email_changed(
        &self,
        new_state: Option<State<EmailObject>>,
    ) -> Result<Option<BackendEvent>> {
        let mut cached_state: State<EmailObject> =
            if let Some(s) = self.store.email_state.lock().await.as_ref() {
                if Some(s) == new_state.as_ref() {
                    return Ok(None);
                }
                s.clone()
            } else {
                return Ok(None);
            };
        let mail_account_id = self.session_guard().await?.mail_account_id()?;
        let mut events = vec![];
        loop {
            let email_changes_call: EmailChanges = EmailChanges::new(
                Changes::<EmailObject>::new()
                    .account_id(mail_account_id.clone())
                    .since_state(cached_state.clone()),
            );

            let mut req = Request::new(self.request_no.clone());
            let prev_seq = req.add_call(&email_changes_call).await;
            req.add_call(&EmailGet::new(
                Get::new()
                    .ids(Some(Argument::reference::<
                        EmailChanges,
                        EmailObject,
                        EmailObject,
                    >(
                        prev_seq,
                        ResultField::<EmailChanges, EmailObject>::new("/created"),
                    )))
                    .account_id(mail_account_id.clone()),
            ))
            .await;
            req.add_call(&EmailGet::new(
                Get::new()
                    .ids(Some(Argument::reference::<
                        EmailChanges,
                        EmailObject,
                        EmailObject,
                    >(
                        prev_seq,
                        ResultField::<EmailChanges, EmailObject>::new("/updated"),
                    )))
                    .account_id(mail_account_id.clone()),
            ))
            .await;

            let res_text = self
                .post_async(None, serde_json::to_string(&req)?)
                .await?
                .text()
                .await?;
            if self.server_conf.trace {
                tracing::trace!("email_since_state(): response {res_text:?}");
            }
            let mut v: MethodResponse = match deserialize_from_str(&res_text) {
                Err(err) => {
                    _ = self.store.online_status.set(None, Err(err.clone())).await;
                    return Err(err);
                }
                Ok(s) => s,
            };
            let mut changes_response = ChangesResponse::<EmailObject>::try_from(v.take_first()?)?;
            if changes_response.new_state == cached_state {
                return Ok(None);
            }
            for destroyed_id in std::mem::take(&mut changes_response.destroyed) {
                if let Some((env_hash, mailbox_hashes)) =
                    self.store.remove_envelope(destroyed_id).await
                {
                    for mailbox_hash in mailbox_hashes {
                        events.push(RefreshEvent {
                            account_hash: self.store.account_hash,
                            mailbox_hash,
                            kind: RefreshEventKind::Remove(env_hash),
                        });
                    }
                }
            }
            let get_response = GetResponse::<EmailObject>::try_from(v.take_first()?)?;

            {
                // Created
                let GetResponse::<EmailObject> { list, .. } = get_response;

                for envobj in list {
                    let mailbox_hashes = envobj
                        .mailbox_ids
                        .iter()
                        .map(|(id, _)| id.into_hash())
                        .collect::<Vec<_>>();
                    let env = self.store.add_envelope(envobj).await;
                    for mailbox_hash in mailbox_hashes {
                        let mut mailboxes_lck = self.store.mailboxes.write().unwrap();
                        mailboxes_lck.entry(mailbox_hash).and_modify(|mbox| {
                            if !env.is_seen() {
                                mbox.unread_emails.lock().unwrap().insert_new(env.hash());
                            }
                            mbox.total_emails.lock().unwrap().insert_new(env.hash());
                        });
                        events.push(RefreshEvent {
                            account_hash: self.store.account_hash,
                            mailbox_hash,
                            kind: RefreshEventKind::Create(Box::new(env.clone())),
                        });
                    }
                }
            }
            let get_response = GetResponse::<EmailObject>::try_from(v.take_first()?)?;

            {
                let reverse_id_store_lck = self.store.reverse_id_store.lock().await;
                // Updated
                let GetResponse::<EmailObject> { list, .. } = get_response;

                let mut mailboxes_lck = self.store.mailboxes.write().unwrap();
                for envobj in list {
                    if let Some(env_hash) = reverse_id_store_lck.get(&envobj.id) {
                        let new_flags = protocol::keywords_to_flags(
                            envobj.keywords().keys().cloned().collect(),
                        );
                        for mailbox_id in envobj.mailbox_ids.keys() {
                            let mailbox_hash = mailbox_id.into_hash();
                            mailboxes_lck.entry(mailbox_hash).and_modify(|mbox| {
                                if new_flags.0.contains(Flag::SEEN) {
                                    mbox.unread_emails.lock().unwrap().remove(*env_hash);
                                } else {
                                    mbox.unread_emails.lock().unwrap().insert_new(*env_hash);
                                }
                            });
                            events.push(RefreshEvent {
                                account_hash: self.store.account_hash,
                                mailbox_hash,
                                kind: RefreshEventKind::NewFlags(*env_hash, new_flags.clone()),
                            });
                        }
                    }
                }
            }
            if !v.method_responses.is_empty() {
                return Err(Error::new(format!(
                    "JMAP server returned {} unexpected extra `methodResponses` entries for an \
                     Email/changes request",
                    v.method_responses.len()
                ))
                .set_kind(ErrorKind::ProtocolError));
            }
            if changes_response.has_more_changes {
                cached_state = changes_response.new_state;
            } else {
                *self.store.email_state.lock().await = Some(changes_response.new_state);

                break;
            }
        }

        Ok(events.try_into().ok())
    }

    pub async fn send_request(&self, request: String) -> Result<String> {
        if self.server_conf.trace {
            tracing::trace!("send_request(): request {:?}", request);
        }
        let res_text = self.post_async(None, request).await?.text().await?;
        if self.server_conf.trace {
            tracing::trace!("send_request(): response {:?}", res_text);
        }
        let _: MethodResponse = match deserialize_from_str(&res_text) {
            Err(err) => {
                tracing::error!("Could not deserialize response {res_text:?}: {err}");
                _ = self.store.online_status.set(None, Err(err.clone())).await;
                return Err(err);
            }
            Ok(s) => s,
        };
        Ok(res_text)
    }

    /// Decide whether `response` to a request sent to `request_url` should be
    /// followed as a redirect.
    ///
    /// Returns `Ok(None)` if the response is not a followable redirect and
    /// should be processed as-is, `Ok(Some(url))` if the request may be
    /// re-issued to `url`, and `Err` if the redirect must not be followed.
    ///
    /// Redirects are only followed when the target has the same origin
    /// (scheme, host and port) as `original_url`. This check happens before
    /// the redirected request is issued, so credentials (Bearer token via
    /// default headers, or Basic via curl engine authentication) can never
    /// reach a different origin.
    fn redirect_target(
        &self,
        original_url: &Url,
        request_url: &Url,
        response: &isahc::Response<isahc::AsyncBody>,
    ) -> Result<Option<Url>> {
        if !is_redirect_status(response.status()) {
            return Ok(None);
        }
        let Some(location) = response.headers().get(http::header::LOCATION) else {
            return Ok(None);
        };
        let location = String::from_utf8_lossy(location.as_bytes());
        let mut target = match Url::parse(&location) {
            Ok(url) => url,
            Err(url::ParseError::RelativeUrlWithoutBase) => request_url
                .join(&location)
                .map_err(|err| self.redirect_error(request_url, &location, &err.to_string()))?,
            Err(err) => return Err(self.redirect_error(request_url, &location, &err.to_string())),
        };
        // Never honor credentials embedded in the redirect target itself.
        _ = target.set_username("");
        _ = target.set_password(None);
        if target.origin() != original_url.origin() {
            return Err(Error::new(format!(
                "JMAP request to {request_url} was redirected to {target}, which is of a \
                 different origin (scheme/host/port) than {}. Refusing to follow the redirect \
                 in order to protect authentication credentials.",
                self.server_conf.server_url
            ))
            .set_kind(ErrorKind::Network(NetworkErrorKind::ProtocolViolation)));
        }
        Ok(Some(target))
    }

    fn redirect_error(&self, request_url: &Url, location: &str, err: &str) -> Error {
        Error::new(format!(
            "Could not resolve redirect location {location:?} of URL {request_url} when \
             connecting to JMAP server {}: {err}",
            self.server_conf.server_url
        ))
        .set_kind(ErrorKind::Network(NetworkErrorKind::ProtocolViolation))
    }

    pub async fn get_async(&self, url: &Url) -> Result<isahc::Response<isahc::AsyncBody>> {
        let mut request_url = url.clone();
        let mut redirects_followed = 0_usize;
        let mut resp = loop {
            let resp = if self.server_conf.trace {
                let res = self.client.get_async(request_url.as_str()).await;
                tracing::trace!("get_async(): url `{}` response {:?}", request_url, res);
                res?
            } else {
                self.client.get_async(request_url.as_str()).await?
            };
            let next_url = match self.redirect_target(url, &request_url, &resp) {
                Ok(None) => break resp,
                Ok(Some(next_url)) => next_url,
                Err(err) => {
                    _ = self
                        .store
                        .online_status
                        .set(Some(Instant::now()), Err(err.clone()))
                        .await;
                    return Err(err);
                }
            };
            redirects_followed += 1;
            if redirects_followed > MAX_REDIRECTS {
                let err = Error::new(format!(
                    "Too many redirects (limit is {MAX_REDIRECTS}) when connecting to JMAP \
                     server endpoint {}",
                    self.server_conf.server_url
                ))
                .set_kind(ErrorKind::Network(NetworkErrorKind::TooManyRedirects));
                _ = self
                    .store
                    .online_status
                    .set(Some(Instant::now()), Err(err.clone()))
                    .await;
                return Err(err);
            }
            request_url = next_url;
        };
        if !resp.status().is_success() {
            let kind: crate::error::NetworkErrorKind = resp.status().into();
            let res_text = resp.text().await.unwrap_or_default();
            let err = Error::new(format!(
                "Could not connect to JMAP server endpoint for {}. Reply from server: {res_text}",
                self.server_conf.server_url
            ))
            .set_kind(kind.into());
            _ = self
                .store
                .online_status
                .set(Some(Instant::now()), Err(err.clone()))
                .await;
            return Err(err);
        }
        Ok(resp)
    }

    pub async fn post_async<T: Into<Vec<u8>> + Send + Sync>(
        &self,
        api_url: Option<&Url>,
        request: T,
    ) -> Result<isahc::Response<isahc::AsyncBody>> {
        let request: Vec<u8> = request.into();
        if self.server_conf.trace {
            tracing::trace!(
                "post_async(): request {:?}",
                String::from_utf8_lossy(&request)
            );
        }
        let original_url = match api_url {
            Some(api_url) => api_url.clone(),
            None => (*self.session_guard().await?.api_url).clone(),
        };
        let mut request_url = original_url.clone();
        let mut redirects_followed = 0_usize;
        // 301/302/303 responses switch the method to GET, mimicking
        // curl/isahc redirect behavior; 307/308 preserve it.
        let mut method_is_get = false;
        let mut resp = loop {
            let resp = if method_is_get {
                self.client.get_async(request_url.as_str()).await?
            } else {
                self.client
                    .post_async(request_url.as_str(), request.clone())
                    .await?
            };
            if self.server_conf.trace {
                tracing::trace!("post_async(): response {resp:?}",);
            }
            let next_url = match self.redirect_target(&original_url, &request_url, &resp) {
                Ok(None) => break resp,
                Ok(Some(next_url)) => next_url,
                Err(err) => {
                    _ = self
                        .store
                        .online_status
                        .set(Some(Instant::now()), Err(err.clone()))
                        .await;
                    return Err(err);
                }
            };
            redirects_followed += 1;
            if redirects_followed > MAX_REDIRECTS {
                let err = Error::new(format!(
                    "Too many redirects (limit is {MAX_REDIRECTS}) when connecting to JMAP \
                     server endpoint {}",
                    self.server_conf.server_url
                ))
                .set_kind(ErrorKind::Network(NetworkErrorKind::TooManyRedirects));
                _ = self
                    .store
                    .online_status
                    .set(Some(Instant::now()), Err(err.clone()))
                    .await;
                return Err(err);
            }
            method_is_get |= matches!(resp.status().as_u16(), 301..=303);
            request_url = next_url;
        };
        if !resp.status().is_success() {
            let kind: crate::error::NetworkErrorKind = resp.status().into();
            let res_text = resp.text().await.unwrap_or_default();
            let err = Error::new(format!(
                "Could not connect to JMAP server endpoint for {}. Reply from server: {res_text}",
                self.server_conf.server_url
            ))
            .set_kind(kind.into());
            _ = self
                .store
                .online_status
                .set(Some(Instant::now()), Err(err.clone()))
                .await;
            return Err(err);
        }
        Ok(resp)
    }
}
