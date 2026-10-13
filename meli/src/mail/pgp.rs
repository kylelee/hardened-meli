/*
 * meli
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
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! Pluggable PGP backends.
//!
//! This module wires the [`PGPBackend`] trait
//! into meli by selecting between the libgpgme-backed implementation and an
//! arbitrary command-line backend (driven by the contrib
//! `pgp-cli-backends/gpg/*.py` scripts by default).

use std::{
    future::Future,
    hash::{Hash, Hasher},
    process::{Command, Stdio},
};

use melib::{
    email::{
        attachment_types::{ContentDisposition, ContentType, MultipartType, Text},
        pgp::{
            self as melib_pgp, Key, LocateKey, PGPBackend, Recipient, ResultFuture, Signature,
            SignaturesMetadata,
        },
        Attachment, AttachmentBuilder,
    },
    error::{Error, ErrorKind, Result, ResultIntoError},
    parser::BytesExt,
    smol,
};

#[cfg(feature = "gpgme")]
use melib::gpgme::Context as GpgmeContext;

use crate::{
    conf::pgp::{PGPBackendCLI, PGPBackendChoice},
    types::File,
};

/// Decrypts a `multipart/encrypted` or a cleartext encrypted message.
pub async fn decrypt(
    backend: PGPBackendInstance,
    a: Attachment,
) -> Result<(SignaturesMetadata, Vec<u8>)> {
    let Attachment {
        content_type:
            ContentType::Multipart {
                kind: MultipartType::Encrypted,
                parts,
                ..
            },
        ..
    } = a
    else {
        if matches!(
            a.content_type,
            ContentType::Text {
                kind: Text::Plain,
                ..
            }
        ) {
            let content = a.text(Text::Plain);
            if content
                .trim_start()
                .starts_with("-----BEGIN PGP MESSAGE-----")
                && content.trim_end().ends_with("-----END PGP MESSAGE-----")
            {
                // Cleartext (PGP message format) — pass to backend for decryption.
                let octet_stream = melib::utils::canonicalize_crlf(content.trim().as_bytes());
                let mut backend = backend;
                return backend.decrypt(&octet_stream)?.await;
            }
        }
        return Err(Error::new("No encrypted payload found").set_kind(ErrorKind::ValueError));
    };
    let blob = parts
        .iter()
        .find(|p| p.content_type == "application/octet-stream")
        .ok_or_else(|| Error::new("No encrypted payload found").set_kind(ErrorKind::ValueError))?;
    let decoded_octet_stream = blob.decode(Default::default());
    let mut backend = backend;
    backend.decrypt(&decoded_octet_stream)?.await
}

pub fn verify(
    backend: PGPBackendInstance,
    a: Attachment,
) -> impl Future<Output = Result<SignaturesMetadata>> + Send {
    use std::collections::{hash_map::DefaultHasher, BTreeMap};
    use std::hash::{Hash, Hasher};
    use std::sync::{Arc, Mutex};

    thread_local! {
        static CACHE: Arc<Mutex<BTreeMap<u64, Result<SignaturesMetadata>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
    }

    let cache = CACHE.with(|cache| cache.clone());
    async move {
        let mut hasher = DefaultHasher::new();
        let unverified_signature = melib_pgp::extract_unverified_signature(&a)
            .chain_err_summary(|| "Could not verify signature.")?;
        match unverified_signature {
            melib_pgp::UnverifiedSignature::Detached {
                signed_part,
                signature,
            } => {
                backend.hash(&mut hasher);
                signed_part.hash(&mut hasher);
                signature.body().hash(&mut hasher);
                let attachment_hash: u64 = hasher.finish();

                {
                    let lck = cache.lock().unwrap();
                    let in_cache: bool = lck.contains_key(&attachment_hash);
                    if in_cache {
                        return lck[&attachment_hash].clone();
                    }
                }

                let mut backend = backend;
                let result = backend.verify(signature.body().trim(), &signed_part)?.await;
                {
                    let mut lck = cache.lock().unwrap();
                    lck.insert(attachment_hash, result.clone());
                }
                result
            }
            melib_pgp::UnverifiedSignature::Cleartext { text } => {
                backend.hash(&mut hasher);
                text.hash(&mut hasher);
                let attachment_hash: u64 = hasher.finish();

                {
                    let lck = cache.lock().unwrap();
                    let in_cache: bool = lck.contains_key(&attachment_hash);
                    if in_cache {
                        return lck[&attachment_hash].clone();
                    }
                }

                let mut backend = backend;
                let result = backend.verify_cleartext(&text)?.await;
                {
                    let mut lck = cache.lock().unwrap();
                    lck.insert(attachment_hash, result.clone());
                }
                result
            }
        }
    }
}

/// Turn raw backend verification output into the per-signature user-facing
/// result, failing closed: an empty signature list is an error, never a
/// silent success.
///
/// CVE-2007-1268 (mutt ≤ 1.5.13) was exactly this class of bug — an
/// unread/missing `GnuPG` status (`--status-fd`) read as "verified",
/// letting unsigned portions ride a valid signature. The gpgme backend
/// already refuses to return empty metadata (`"No signatures found."`);
/// the CLI backend trusts a user script's JSON, and a script answering
/// `[]` must not mark a `multipart/signed` container `SignedVerified`
/// with no signature at all.
pub fn signatures_into_error(metadata: SignaturesMetadata) -> Result<Option<String>> {
    let mut comment = String::new();

    if metadata.signatures.is_empty() {
        return Err(Error::new("No signatures found.").set_kind(ErrorKind::NotFound));
    }

    for sig in metadata.signatures.into_iter().rev() {
        let Signature {
            summary,
            cert:
                Recipient {
                    keyid: fingerprint,
                    status,
                },
            validity,
            validity_reason,
            cleartext,
        } = sig;
        if let Err(err) = status {
            return Err(Error::new(format!("BAD signature from {fingerprint}")).set_source(Some(Box::new(err)))
                .set_kind(ErrorKind::ValueError));
        }
        if cleartext {
            comment = format!("{comment}[SAFETY WARNING: Cleartext signature!]");
        }
        if let Some(validity_reason) = validity_reason {
            comment =
                format!("{comment}good signature by {fingerprint}:{summary}{validity_reason}\n");
        } else {
            let validity = validity.string_representation();
            comment = format!(
                "{comment}good signature by {fingerprint}{colon}{summary}[{validity}]\n",
                colon = if summary.is_empty() { "" } else { ":" }
            );
        }
    }
    if comment.ends_with('\n') {
        comment.pop();
    }

    if comment.is_empty() {
        return Ok(None);
    }

    Ok(Some(comment))
}

pub fn sign_filter(
    choice: PGPBackendChoice,
    default_key: Option<String>,
    mut sign_keys: Vec<Key>,
) -> Result<impl FnOnce(AttachmentBuilder) -> crate::mail::AttachmentBoxFuture + Send> {
    Ok(
        move |a: AttachmentBuilder| -> crate::mail::AttachmentBoxFuture {
            Box::pin(async move {
                let mut backend = choice.instantiate()?;
                if let Some(default_key) = default_key {
                    backend.set_auto_key_locate(LocateKey::LOCAL)?;
                    let keys = backend.keylist(false, Some(default_key.clone()))?.await?;
                    if keys.is_empty() {
                        return Err(Error::new(format!(
                            "Could not locate sign key with ID `{default_key}`"
                        )));
                    }
                    sign_keys.extend(keys);
                }
                if sign_keys.is_empty() {
                    return Err(Error::new(
                        "No key was selected for signing; please select one.",
                    ));
                }
                let a: Attachment = a.into();
                let raw = a.into_raw();
                let signed_data = melib::utils::canonicalize_crlf(raw.as_bytes());
                let (sig_metadata, sig_bytes) =
                    backend.sign(sign_keys, &signed_data, false)?.await?;
                let sig_attachment =
                    Attachment::new(ContentType::PGPSignature, Default::default(), sig_bytes);
                let a: AttachmentBuilder = a.into();
                let parts = vec![a, sig_attachment.into()];
                let boundary = ContentType::make_boundary(&parts);

                let micalg = sig_metadata.micalg().into_bytes();
                Ok(Attachment::new(
                    ContentType::Multipart {
                        boundary: boundary.into_bytes(),
                        kind: MultipartType::Signed,
                        parts: parts.into_iter().map(|a| a.into()).collect::<Vec<_>>(),
                        parameters: vec![
                            (b"micalg".into(), micalg),
                            (b"protocol".into(), b"\"application/pgp-signature\"".into()),
                        ],
                    },
                    Default::default(),
                    vec![],
                )
                .into())
            })
        },
    )
}

/// Encrypt (and optionally sign) an outgoing message body.
///
/// `hidden_recipients` is forwarded to [`PGPBackend::encrypt`] and controls
/// whether the backend must suppress the recipient key IDs in the ciphertext
/// (mutt's `--hidden-recipient` / gpg's `--throw-keyids`). The composer sets it
/// whenever the draft carries a `Bcc` header (see
/// [`draft_has_bcc_recipients`]), because otherwise any `To`/`Cc` recipient
/// could run `gpg --list-packets` on the ciphertext, read the `Bcc` recipient's
/// key ID from the PKESK packets and infer that a hidden recipient exists —
/// CVE-2024-49395.
pub fn encrypt_filter(
    choice: PGPBackendChoice,
    encrypt_for_self: Option<melib::Address>,
    default_sign_key: Option<String>,
    mut sign_keys: Option<Vec<Key>>,
    default_encrypt_key: Option<String>,
    mut encrypt_keys: Vec<Key>,
    hidden_recipients: bool,
) -> Result<impl FnOnce(AttachmentBuilder) -> crate::mail::AttachmentBoxFuture + Send> {
    Ok(
        move |a: AttachmentBuilder| -> crate::mail::AttachmentBoxFuture {
            Box::pin(async move {
                let mut backend = choice.instantiate()?;
                if let Some(default_key) = default_sign_key {
                    backend.set_auto_key_locate(LocateKey::LOCAL)?;
                    let keys = backend.keylist(true, Some(default_key.clone()))?.await?;
                    if keys.is_empty() {
                        return Err(Error::new(format!(
                            "Could not locate sign key with ID `{default_key}`"
                        )));
                    }
                    if let Some(ref mut sign_keys) = sign_keys {
                        sign_keys.extend(keys);
                    } else {
                        sign_keys = Some(keys);
                    }
                }
                if let Some(ref sign_keys) = sign_keys {
                    if sign_keys.is_empty() {
                        return Err(Error::new(
                            "No key was selected for signing; please select one.",
                        ));
                    }
                }
                if let Some(default_key) = default_encrypt_key {
                    backend.set_auto_key_locate(LocateKey::LOCAL)?;
                    let keys = backend.keylist(false, Some(default_key.clone()))?.await?;
                    if keys.is_empty() {
                        return Err(Error::new(format!(
                            "Could not locate encryption key with ID `{default_key}`"
                        )));
                    }
                    encrypt_keys.extend(keys);
                }
                if encrypt_keys.is_empty() {
                    return Err(Error::new(
                        "No key was selected for encryption; please select one.",
                    ));
                }
                if let Some(encrypt_for_self) = encrypt_for_self {
                    backend.set_auto_key_locate(LocateKey::LOCAL)?;
                    let keys = backend
                        .keylist(false, Some(encrypt_for_self.to_string()))?
                        .await?;
                    if keys.is_empty() {
                        return Err(Error::new(format!(
                            "Could not locate personal encryption key for address \
                         `{encrypt_for_self}`"
                        )));
                    }
                    for key in keys {
                        if !encrypt_keys.contains(&key) {
                            encrypt_keys.push(key);
                        }
                    }
                }
                let a: Attachment = if let Some(sign_keys) = sign_keys {
                    let a: Attachment = a.into();
                    let raw = a.into_raw();
                    let data = melib::utils::canonicalize_crlf(raw.as_bytes());
                    let (sig_metadata, sig_bytes) = backend.sign(sign_keys, &data, false)?.await?;
                    let sig_attachment =
                        Attachment::new(ContentType::PGPSignature, Default::default(), sig_bytes);
                    let a: AttachmentBuilder = a.into();
                    let parts = vec![a, sig_attachment.into()];
                    let boundary = ContentType::make_boundary(&parts);
                    let micalg = sig_metadata.micalg().into_bytes();
                    Attachment::new(
                        ContentType::Multipart {
                            boundary: boundary.into_bytes(),
                            kind: MultipartType::Signed,
                            parts: parts.into_iter().map(|a| a.into()).collect::<Vec<_>>(),
                            parameters: vec![
                                (b"micalg".into(), micalg),
                                (b"protocol".into(), b"\"application/pgp-signature\"".into()),
                            ],
                        },
                        Default::default(),
                        vec![],
                    )
                } else {
                    a.into()
                };
                let data = a.into_raw().into_bytes();

                let enc_attachment = {
                    let mut a = Attachment::new(
                        ContentType::OctetStream {
                            name: None,
                            parameters: vec![],
                        },
                        Default::default(),
                        backend
                            .encrypt(
                                encrypt_keys,
                                sign_keys.unwrap_or_default(),
                                &data,
                                hidden_recipients,
                            )?
                            .await?,
                    );
                    a.content_disposition =
                        ContentDisposition::from(br#"attachment; filename="msg.asc""#);
                    a
                };
                let mut a: AttachmentBuilder = AttachmentBuilder::new(b"Version: 1\n");

                a.set_content_type_from_bytes(b"application/pgp-encrypted");
                a.set_content_disposition(ContentDisposition::from(b"attachment"));
                let parts = vec![a, enc_attachment.into()];
                let boundary = ContentType::make_boundary(&parts);
                Ok(Attachment::new(
                    ContentType::Multipart {
                        boundary: boundary.into_bytes(),
                        kind: MultipartType::Encrypted,
                        parts: parts.into_iter().map(|a| a.into()).collect::<Vec<_>>(),
                        parameters: vec![(
                            b"protocol".into(),
                            b"\"application/pgp-encrypted\"".into(),
                        )],
                    },
                    Default::default(),
                    vec![],
                )
                .into())
            })
        },
    )
}

/// Returns `true` when `draft` carries a non-empty `Bcc` header.
///
/// CVE-2024-49395: a `Bcc` recipient must be part of the encryption set — it
/// has to be able to decrypt the message — but its key ID must not be
/// recoverable from the ciphertext, otherwise any `To`/`Cc` recipient can infer
/// the hidden recipient from the PKESK packets. The composer uses this
/// predicate to enable hidden-recipient encryption for exactly those drafts.
pub fn draft_has_bcc_recipients(draft: &melib::Draft) -> bool {
    draft
        .headers()
        .get(melib::HeaderName::BCC)
        .is_some_and(|bcc| !bcc.trim().is_empty())
}

impl PGPBackendChoice {
    #[inline]
    pub fn instantiate(&self) -> Result<PGPBackendInstance> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME => Ok(PGPBackendInstance::GpgME {
                ctx: GpgmeContext::new()?,
            }),
            #[cfg(not(feature = "gpgme"))]
            Self::GpgME => Err(Error::new(
                "Cannot instantiate GpgME backend: meli must be compiled with libgpgme. Try \
                 choosing another PGP backend.",
            )
            .set_kind(ErrorKind::Configuration)),
            #[cfg(feature = "sequoia")]
            Self::Sequoia => Ok(PGPBackendInstance::Sequoia {
                ctx: melib::sequoia::Context::new()?,
            }),
            #[cfg(not(feature = "sequoia"))]
            Self::Sequoia => Err(Error::new(
                "Cannot instantiate Sequoia backend: meli must be compiled with sequoia. Try \
                 choosing another PGP backend.",
            )
            .set_kind(ErrorKind::Configuration)),
            Self::CLI(cli) => Ok(PGPBackendInstance::CLI {
                auto_key_locate: cli.auto_key_locate,
                cli: cli.clone(),
            }),
        }
    }
}

/// Owned, `'static` PGP backend instance.
pub enum PGPBackendInstance {
    #[cfg(feature = "gpgme")]
    GpgME { ctx: GpgmeContext },
    #[cfg(feature = "sequoia")]
    Sequoia { ctx: melib::sequoia::Context },
    CLI {
        auto_key_locate: LocateKey,
        cli: PGPBackendCLI,
    },
}

impl Hash for PGPBackendInstance {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => ctx.hash(state),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => ctx.hash(state),
            Self::CLI { cli, .. } => cli.hash(state),
        }
    }
}

impl PGPBackend for PGPBackendInstance {
    fn set_auto_key_locate(&mut self, val: LocateKey) -> Result<()> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => {
                PGPBackend::set_auto_key_locate(ctx, val)?;
                Ok(())
            }
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => {
                PGPBackend::set_auto_key_locate(ctx, val)?;
                Ok(())
            }
            Self::CLI {
                auto_key_locate, ..
            } => {
                *auto_key_locate = val;
                Ok(())
            }
        }
    }

    fn get_auto_key_locate(&self) -> Result<LocateKey> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::get_auto_key_locate(ctx),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => PGPBackend::get_auto_key_locate(ctx),
            Self::CLI {
                auto_key_locate, ..
            } => Ok(*auto_key_locate),
        }
    }

    fn get_key(&self, secret: bool, pattern: String) -> ResultFuture<Key> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::get_key(ctx, secret, pattern),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => PGPBackend::get_key(ctx, secret, pattern),
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                let get_key_command = cli.get_key_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let auto_key_locate_str = auto_key_locate.clone();
                        let mut envs = vec![("AUTO_KEY_LOCATE", auto_key_locate_str.as_str())];
                        if secret {
                            envs.push(("SECRET", ""));
                        }
                        let output = Command::new(&get_key_command)
                            .envs(envs)
                            .arg(&pattern)
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .chain_err_summary(|| format!("Could not launch {get_key_command}"))?;
                        if !output.status.success() {
                            return Err(format!("{get_key_command} exited with {output:?}").into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        Ok(
                            serde_json::from_slice::<Key>(&output.stdout).map_err(|err| {
                                format!(
                                    "Could not deserialize key response from \
                                 {get_key_command}: {err}"
                                )
                            })?,
                        )
                    })
                    .await
                }))
            }
        }
    }

    fn verify(&mut self, signature: &[u8], text: &[u8]) -> ResultFuture<SignaturesMetadata> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::verify(ctx, signature, text),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => PGPBackend::verify(ctx, signature, text),
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                let verify_command = cli.verify_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                let signature = File::create_temp_file(signature, None, None, None, true)?;
                let text = File::create_temp_file(text, None, None, None, true)?;
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let output = Command::new(&verify_command)
                            .arg(signature.path())
                            .arg(text.path())
                            .env("AUTO_KEY_LOCATE", auto_key_locate.clone())
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .chain_err_summary(|| format!("Could not launch {verify_command}"))?;
                        if !output.status.success() {
                            return Err(format!("{verify_command} exited with {output:?}").into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        let signatures: Vec<Signature> = serde_json::from_slice(&output.stdout)
                            .map_err(|err| {
                                format!(
                                    "Could not deserialize signature response from \
                                     {verify_command}: {err}"
                                )
                            })?;
                        Ok(SignaturesMetadata { signatures })
                    })
                    .await
                }))
            }
        }
    }

    fn verify_cleartext(&mut self, text: &[u8]) -> ResultFuture<SignaturesMetadata> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::verify_cleartext(ctx, text),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => PGPBackend::verify_cleartext(ctx, text),
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                let verify_command = cli.verify_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                let text = File::create_temp_file(text, None, None, None, true)?;
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let output = Command::new(&verify_command)
                            .arg(text.path())
                            .env("AUTO_KEY_LOCATE", auto_key_locate.clone())
                            .env("CLEARTEXT", "")
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .chain_err_summary(|| format!("Could not launch {verify_command}"))?;
                        if !output.status.success() {
                            return Err(format!("{verify_command} exited with {output:?}").into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        let signatures: Vec<Signature> = serde_json::from_slice(&output.stdout)
                            .map_err(|err| {
                                format!(
                                    "Could not deserialize signature response from \
                                     {verify_command}: {err}"
                                )
                            })?;
                        Ok(SignaturesMetadata { signatures })
                    })
                    .await
                }))
            }
        }
    }

    fn keylist(&self, secret: bool, pattern: Option<String>) -> ResultFuture<Vec<Key>> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::keylist(ctx, secret, pattern),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => PGPBackend::keylist(ctx, secret, pattern),
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                let keylist_command = cli.keylist_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let auto_key_locate_str = auto_key_locate.clone();
                        let mut envs = vec![("AUTO_KEY_LOCATE", auto_key_locate_str.as_str())];
                        if secret {
                            envs.push(("SECRET", ""));
                        }
                        let mut cmd = Command::new(&keylist_command);
                        if let Some(ref pattern) = pattern {
                            cmd.arg(pattern);
                        }
                        let output = cmd
                            .envs(envs)
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .chain_err_summary(|| format!("Could not launch {keylist_command}"))?;
                        if !output.status.success() {
                            return Err(format!("{keylist_command} exited with {output:?}").into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        Ok(
                            serde_json::from_slice::<Vec<Key>>(&output.stdout).map_err(|err| {
                                format!(
                                    "Could not deserialize keys response from {keylist_command}: \
                                 {err}"
                                )
                            })?,
                        )
                    })
                    .await
                }))
            }
        }
    }

    fn sign(
        &mut self,
        sign_keys: Vec<Key>,
        text: &[u8],
        is_binary: bool,
    ) -> ResultFuture<(melib::email::pgp::NewSignature, Vec<u8>)> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::sign(ctx, sign_keys, text, is_binary),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => PGPBackend::sign(ctx, sign_keys, text, is_binary),
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                let sign_command = cli.sign_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                let text = File::create_temp_file(text, None, None, None, true)?;
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let mut cmd = Command::new(&sign_command);
                        cmd.env("AUTO_KEY_LOCATE", auto_key_locate.clone())
                            .arg(text.path());
                        if is_binary {
                            cmd.env("IS_BINARY", "");
                        }
                        for key in sign_keys {
                            cmd.arg(&key.fingerprint);
                        }
                        let output = cmd
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .chain_err_summary(|| format!("Could not launch {sign_command}"))?;
                        if !output.status.success() {
                            return Err(format!("{sign_command} exited with {output:?}").into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        use serde::de::Deserialize;
                        Ok(
                            serde_json::from_slice::<[serde_json::Value; 2]>(&output.stdout)
                                .and_then(|[n, b]| {
                                    Ok((
                                        melib::email::pgp::NewSignature::deserialize(n)?,
                                        <Vec<u8>>::deserialize(b)?,
                                    ))
                                })
                                .map_err(|err| {
                                    format!(
                                        "Could not deserialize new signature response from \
                                     {sign_command}: {err}"
                                    )
                                })?,
                        )
                    })
                    .await
                }))
            }
        }
    }

    fn encrypt(
        &mut self,
        encrypt_keys: Vec<Key>,
        sign_keys: Vec<Key>,
        plain: &[u8],
        hidden_recipients: bool,
    ) -> ResultFuture<Vec<u8>> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => {
                PGPBackend::encrypt(ctx, encrypt_keys, sign_keys, plain, hidden_recipients)
            }
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => {
                PGPBackend::encrypt(ctx, encrypt_keys, sign_keys, plain, hidden_recipients)
            }
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                // CLI 后端契约（gpg_encrypt.py）只做加密：sign_keys 由调用
                // 方在 encrypt_filter 里以先签后密的方式组装，这里忽略。
                _ = sign_keys;
                let encrypt_command = cli.encrypt_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                let plain = File::create_temp_file(plain, None, None, None, true)?;
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let mut cmd = Command::new(&encrypt_command);
                        // CLI backend contract, following the `AUTO_KEY_LOCATE`
                        // precedent: `HIDDEN_RECIPIENTS=1` tells the script to
                        // use `--hidden-recipient`/`--throw-keyids` for the
                        // recipient set so that the ciphertext does not reveal
                        // the Bcc recipients (CVE-2024-49395).
                        cmd.env("AUTO_KEY_LOCATE", auto_key_locate.clone())
                            .env(
                                "HIDDEN_RECIPIENTS",
                                if hidden_recipients { "1" } else { "0" },
                            )
                            .arg(plain.path());
                        for key in encrypt_keys {
                            cmd.arg(&key.fingerprint);
                        }
                        let output = cmd
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .chain_err_summary(|| format!("Could not launch {encrypt_command}"))?;
                        if !output.status.success() {
                            return Err(format!("{encrypt_command} exited with {output:?}").into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        Ok(
                            serde_json::from_slice::<Vec<u8>>(&output.stdout).map_err(|err| {
                                format!(
                                    "Could not deserialize encryption response from \
                                 {encrypt_command}: {err}"
                                )
                            })?,
                        )
                    })
                    .await
                }))
            }
        }
    }

    fn decrypt(&mut self, cipher: &[u8]) -> ResultFuture<(SignaturesMetadata, Vec<u8>)> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::decrypt(ctx, cipher),
            #[cfg(feature = "sequoia")]
            Self::Sequoia { ctx } => PGPBackend::decrypt(ctx, cipher),
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                let decrypt_command = cli.decrypt_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                let cipher = File::create_temp_file(cipher, None, None, None, true)?;
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let output = Command::new(&decrypt_command)
                            .env("AUTO_KEY_LOCATE", auto_key_locate.clone())
                            .arg(cipher.path())
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .output()
                            .chain_err_summary(|| format!("Could not launch {decrypt_command}"))?;
                        if !output.status.success() {
                            // CVE-2017-17688 (EFAIL) regression: GnuPG
                            // streams decrypted literal data before the MDC
                            // verdict, so a failing decrypt command's captured
                            // stdout can carry partial (tampered) plaintext.
                            // The error surfaced to the mail view must never
                            // embed the process output byte-for-byte.
                            return Err(format!(
                                "{decrypt_command} failed with exit status {}",
                                output
                                    .status
                                    .code()
                                    .map_or_else(|| "unknown".to_string(), |code| code.to_string())
                            )
                            .into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        // 解密命令的 JSON 契约只承载明文（`data`）；脚本
                        // 不做验签，也不翻译任何状态行（CVE-2018-12020），
                        // 因此元数据恒为空——多余键一律忽略。
                        #[derive(Deserialize)]
                        struct Out {
                            data: Vec<u8>,
                        }
                        let out: Out = serde_json::from_slice(&output.stdout).map_err(|err| {
                            format!(
                                "Could not deserialize decryption response from \
                                 {decrypt_command}: {err}"
                            )
                        })?;
                        Ok((SignaturesMetadata::default(), out.data))
                    })
                    .await
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use melib::{
        email::pgp::{Summary, Validity},
        Draft, HeaderName,
    };

    use super::*;
    use crate::conf::pgp::PGPBackendChoice;

    /// CVE-2024-49395: a `Bcc` header with at least one address must enable
    /// hidden-recipient encryption. Multiple addresses count as well.
    #[test]
    fn draft_with_bcc_address_is_detected() {
        let mut draft = Draft::default();
        draft.set_header(HeaderName::BCC, "bcc-49395@cve.example".to_string());
        assert!(draft_has_bcc_recipients(&draft));

        let mut draft = Draft::default();
        draft.set_header(
            HeaderName::BCC,
            "First <bcc-1@cve.example>, second@cve.example".to_string(),
        );
        assert!(draft_has_bcc_recipients(&draft));
    }

    /// No `Bcc` header at all, and an explicitly empty value, must both leave
    /// hidden-recipient encryption off.
    #[test]
    fn draft_without_bcc_header_is_not_detected() {
        let draft = Draft::default();
        assert!(!draft_has_bcc_recipients(&draft));

        let mut draft = Draft::default();
        draft.set_header(HeaderName::BCC, String::new());
        assert!(!draft_has_bcc_recipients(&draft));
    }

    /// A whitespace-only `Bcc` value is treated as absent.
    #[test]
    fn draft_with_whitespace_only_bcc_is_not_detected() {
        let mut draft = Draft::default();
        draft.set_header(HeaderName::BCC, "   ".to_string());
        assert!(!draft_has_bcc_recipients(&draft));
    }

    // Keys generated with <https://github.com/epilys/gen-rfc9500-gpg-keys>

    const PUBKEY: &[u8] = br"-----BEGIN PGP PUBLIC KEY BLOCK-----

xsBNBAAAAAABCACw+egZQ6eumJKq3hfKfED4dE/tL4FI5sjqont9ABVI+1GSqyi1
bFBgsRjM0THllIdMbKmJtWwnKW8J+5OgNN8y6Xxv8JmM/Y5vQt2lis0fqXmG8UTz
0VTWdlAXXmhUs6lSADvAaIe4RVrCsZ97L3ZQTryY7JRVcbB4khUN3Gp0yg+801SX
zoFTTa+UGIRLE66jH51aa5VXu99hnv1OiH8tQrjdi8mH6uG/icq4XuIeNWMF32wH
qIOOPvQcWV3M5D2vxJEj702Ku6k9OQXkAo17qRSEonWW4HtLbtmS8He1JNPc/n3d
VUm+fM6NoDXPoLP7j55G9zKyqGtGAWXAj1MTABEBAAHCwL4EHwEKAHIFgmqwA58J
EMwuljyZl1FjRxQAAAAAAB4AIHNhbHRAbm90YXRpb25zLnNlcXVvaWEtcGdwLm9y
Z6O5Y4S0HyXDKFGauzW0rj+twitM3GHMI19Yx6dcilxCApsnFiEErtwR+84tdGv4
v3FmzC6WPJmXUWMAAOXvCACZUwjphdH7a0/NaQpgEUZ9ivliZIOra9FyNB6Yonc9
wFnlkE7+V0XfiVyZin1rs35at3B81cRBzHGqF4AzhkbYH3yEfzFZ4BBZn3osqH+o
BFJWzYfHsmV6FRbNOVOUzhQyA1g0bo2QbdRwaUWu9Wcvn+kdQ9x2mc+bmpm8Rm0M
MbomWKpmh/56dJCqRUn7ybQQYjfZQQGiCLpkLY+sP6S9Lx4D7xlHO9RkvUKWSQ7F
kKIzqZTr4H6gdXuZOaTzDq1ZPOclVJlRrwVcLwWjNanEfIrWVZe6J1wX98IXoY+o
K/JIs56XjMNodrih7I2so3EUITV1c4pZ0mhN0J9WWJoszRB1c2VyQGV4YW1wbGUu
b3JnwsC7BBMBCgBvBYIAAAAACRDMLpY8mZdRY0cUAAAAAAAeACBzYWx0QG5vdGF0
aW9ucy5zZXF1b2lhLXBncC5vcmehx6kpeoqLZCvewYfbkx/DcWvhW61UtbUcnD2o
4dFa2xYhBK7cEfvOLXRr+L9xZswuljyZl1FjAACCNwgAoTYd6zE8dTfXdj/Ga57N
/XVWONJWc+gW0a2Bt9u76OZTs1H38sg7gFbp/Oxzz27mTrZ/87349IwzujdsGRkQ
nCf93NeOEa4Ts0DsqzZyc6uKBwvlKaHxBMnLoswV8oosrkyZzmw37EZxI75OqJJK
xy1i2v2xBF0WQ8xKs8cCxt3XoDs4sD0eBnXRhyHYpF3pIux4klDd4Bi7cklj5q5v
QDVHRF4NA6RAwd1MXagYUmaYRmokv/yt/mMN/kSppa7WetQ9MYJtAH+BOCVi5dhn
fbOuVxt+TMw6QHT2qpTiIHs1pNBleIth4rAev7g+9KUgkWKNtSAQCZZOcx7I+/Uv
SQ==
=vXlr
-----END PGP PUBLIC KEY BLOCK-----
";

    const PRIVKEY: &[u8] = br"-----BEGIN PGP PRIVATE KEY BLOCK-----

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
5LiEsB1TygJ+T0kBbxm1T12mT20r+kTJV97FUA==
=F94H
-----END PGP PRIVATE KEY BLOCK-----
";

    /// Sequoia 后端只能通过 `SEQUOIA_HOME` 环境变量重定向 keystore；
    /// 进程级互斥 + 保存/恢复，避免与并行测试的其它环境读取相互干扰。
    fn sequoia_test_env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// 上游 `run_gpg_and_sequoia` 的 fork 适配版：gpgme 侧用 per-context
    /// scratch keyring（cve 套件的既有模式，不动进程环境），sequoia 侧用
    /// `SEQUOIA_HOME` 重定向 keystore；gpgme/libgpgme 不可用时优雅跳过。
    #[cfg(all(feature = "sequoia", feature = "gpgme"))]
    #[test]
    fn test_sequoia_sign_verify_encrypt_decrypt_roundtrip() {
        use std::borrow::Cow;
        use std::ffi::CString;

        const FINGERPRINT: &str = "AEDC11FBCE2D746BF8BF7166CC2E963C99975163";

        let _env_guard = sequoia_test_env_lock();
        let tempdir = tempfile::tempdir().unwrap();
        // 保存并在测试结束时恢复 `SEQUOIA_HOME`。
        let old_sequoia_home = std::env::var_os("SEQUOIA_HOME");
        struct RestoreGuard(Option<std::ffi::OsString>);
        impl Drop for RestoreGuard {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(v) => std::env::set_var("SEQUOIA_HOME", v),
                    None => std::env::remove_var("SEQUOIA_HOME"),
                }
            }
        }
        let _restore = RestoreGuard(old_sequoia_home);
        std::env::set_var("SEQUOIA_HOME", tempdir.path());

        let mut gpgme_ctx = match melib::gpgme::Context::new() {
            Ok(v) => v,
            Err(err) if err.kind.is_not_found() => {
                eprintln!("skipping: libgpgme could not be loaded on this host");
                return;
            }
            err => err.unwrap(),
        };
        {
            use melib::gpgme::Protocol;
            let engine_file = gpgme_ctx
                .engine_info()
                .unwrap()
                .into_iter()
                .find(|eng| eng.protocol == Protocol::OpenPGP)
                .and_then(|eng| eng.file_name)
                .unwrap_or_else(|| {
                    panic!("gpgme reports no OpenPGP engine; cannot run this test");
                });
            gpgme_ctx
                .set_engine_info(
                    Protocol::OpenPGP,
                    Some(Cow::Owned(CString::new(engine_file).unwrap())),
                    Some(Cow::Owned(
                        CString::new(tempdir.path().display().to_string()).unwrap(),
                    )),
                )
                .unwrap();
        }

        // gpgme 侧导入公私钥并取回公钥（Key 形态，供 sign/encrypt 过滤器用）
        gpgme_ctx
            .import_key(gpgme_ctx.new_data_mem(PUBKEY).unwrap())
            .unwrap();
        let pubkey: Key = gpgme_ctx.get_key(false, FINGERPRINT).unwrap().into();
        assert_eq!(pubkey.fingerprint, FINGERPRINT);
        gpgme_ctx
            .import_key(gpgme_ctx.new_data_mem(PRIVKEY).unwrap())
            .unwrap();

        // sequoia 侧导入私钥并用本地信任根完成 PKI 认证（否则验签只有
        // marginal/unknown 有效性）。
        {
            let sq = melib::sequoia::Context::new().unwrap();
            let sq = sq.sq;
            let sec_key = String::from_utf8_lossy(PRIVKEY).parse().unwrap();
            let local_trust_root = sq.local_trust_root().unwrap();
            let outcome = sq
                .pki_certify()
                .certify(
                    &local_trust_root,
                    &sec_key,
                    &sec_key
                        .userids()
                        .map(|u| u.component().clone())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            sq.key_import().import_key(outcome.into_cert()).unwrap();
            let _ = sq
                .lookup()
                .lookup_one(
                    FINGERPRINT
                        .parse::<melib::sequoia::sqz::openpgp::Fingerprint>()
                        .unwrap(),
                )
                .unwrap();
        }

        // 签名并验签
        {
            let mut draft = Draft::default();
            draft.set_body("foobar\r\n\r\n".into());
            draft
                .try_set_header("From", "user@example.org".into())
                .unwrap();
            draft
                .try_set_header("To", "user@example.org".into())
                .unwrap();

            let body_attachment: AttachmentBuilder = Attachment::new(
                ContentType::default(),
                Default::default(),
                std::mem::take(&mut draft.body).into_bytes(),
            )
            .into();

            let body: AttachmentBuilder = smol::block_on((sign_filter(
                PGPBackendChoice::Sequoia,
                None,
                vec![pubkey.clone()],
            )
            .unwrap())(body_attachment))
            .unwrap();
            draft.attachments.insert(0, body);
            let raw_mail = draft.finalise().unwrap();
            let mail = melib::Mail::new(raw_mail.into_bytes(), None).expect("Could not parse mail");
            let signatures = smol::block_on(verify(
                PGPBackendChoice::Sequoia.instantiate().unwrap(),
                mail.body(),
            ))
            .unwrap();
            assert_eq!(
                signatures_into_error(signatures).unwrap().unwrap(),
                format!("good signature by {FINGERPRINT}:VALID[f]")
            );
        }

        // 一体化签后加密并解密验签
        {
            let mut draft = Draft::default();
            draft.set_body("foobar\r\n\r\n".into());
            draft
                .try_set_header("From", "user@example.org".into())
                .unwrap();
            draft
                .try_set_header("To", "user@example.org".into())
                .unwrap();

            let body_attachment: AttachmentBuilder = Attachment::new(
                ContentType::default(),
                Default::default(),
                std::mem::take(&mut draft.body).into_bytes(),
            )
            .into();
            let body: AttachmentBuilder = smol::block_on((encrypt_filter(
                PGPBackendChoice::Sequoia,
                None,
                Some(FINGERPRINT.to_string()),
                None,
                None,
                vec![pubkey],
                false,
            )
            .unwrap())(body_attachment))
            .unwrap();

            draft.attachments.insert(0, body);
            let raw_mail = draft.finalise().unwrap();
            let mail = melib::Mail::new(raw_mail.into_bytes(), None).expect("Could not parse mail");

            let (metadata, decrypted) = smol::block_on(decrypt(
                PGPBackendChoice::Sequoia.instantiate().unwrap(),
                mail.body(),
            ))
            .expect("Could not decrypt email");
            assert_eq!(
                metadata,
                SignaturesMetadata {
                    signatures: vec![Signature {
                        summary: Summary::VALID,
                        cert: Recipient {
                            keyid: FINGERPRINT.to_string(),
                            status: Ok(())
                        },
                        validity: Validity::Full,
                        validity_reason: None,
                        cleartext: false
                    }]
                }
            );
            let decrypted = AttachmentBuilder::new(&decrypted).build();
            let signatures = smol::block_on(verify(
                PGPBackendChoice::Sequoia.instantiate().unwrap(),
                decrypted,
            ))
            .unwrap();
            assert_eq!(
                signatures_into_error(signatures).unwrap().unwrap(),
                format!("good signature by {FINGERPRINT}:VALID[f]")
            );
        }
    }
}
