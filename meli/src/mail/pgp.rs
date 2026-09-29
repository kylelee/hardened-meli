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
//! This module wires the [`PGPBackend`](melib::email::pgp::PGPBackend) trait
//! into meli by selecting between the libgpgme-backed implementation and an
//! arbitrary command-line backend (driven by the contrib
//! `pgp-cli-backends/gpg/*.py` scripts by default).

use std::{
    future::Future,
    process::{Command, Stdio},
};

use melib::{
    email::{
        attachment_types::{ContentDisposition, ContentType, MultipartType, Text},
        pgp::{
            self as melib_pgp, DecryptionMetadata, Key, LocateKey, PGPBackend, Recipient,
            ResultFuture, Signature, SignaturesMetadata,
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
) -> Result<(DecryptionMetadata, Vec<u8>)> {
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
                let trimmed = content.trim().as_bytes().to_vec();
                let mut backend = backend;
                return backend.decrypt(&trimmed)?.await;
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

pub fn signatures_into_error(metadata: SignaturesMetadata) -> Result<Option<String>> {
    let mut comment = String::new();

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
            return Err(Error::new(format!("BAD signature from {fingerprint}"))
                .set_source(Some(melib::src_err_arc_wrap! { err }))
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
                let signed_data =
                    melib_pgp::convert_attachment_to_rfc_spec(a.into_raw().as_bytes());
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

pub fn encrypt_filter(
    choice: PGPBackendChoice,
    encrypt_for_self: Option<melib::Address>,
    default_sign_key: Option<String>,
    mut sign_keys: Option<Vec<Key>>,
    default_encrypt_key: Option<String>,
    mut encrypt_keys: Vec<Key>,
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
                    let data = melib_pgp::convert_attachment_to_rfc_spec(a.into_raw().as_bytes());
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
                        backend.encrypt(encrypt_keys, &data)?.await?,
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
    CLI {
        auto_key_locate: LocateKey,
        cli: PGPBackendCLI,
    },
}

impl PGPBackend for PGPBackendInstance {
    fn set_auto_key_locate(&mut self, val: LocateKey) -> Result<()> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => {
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
            Self::CLI {
                auto_key_locate, ..
            } => Ok(*auto_key_locate),
        }
    }

    fn get_key(&self, secret: bool, pattern: String) -> ResultFuture<Key> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::get_key(ctx, secret, pattern),
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

    fn encrypt(&mut self, encrypt_keys: Vec<Key>, plain: &[u8]) -> ResultFuture<Vec<u8>> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::encrypt(ctx, encrypt_keys, plain),
            Self::CLI {
                auto_key_locate,
                cli,
                ..
            } => {
                let encrypt_command = cli.encrypt_command.to_string();
                let auto_key_locate = auto_key_locate.to_string();
                let plain = File::create_temp_file(plain, None, None, None, true)?;
                Ok(Box::pin(async move {
                    smol::unblock(move || {
                        let mut cmd = Command::new(&encrypt_command);
                        cmd.env("AUTO_KEY_LOCATE", auto_key_locate.clone())
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

    fn decrypt(&mut self, cipher: &[u8]) -> ResultFuture<(DecryptionMetadata, Vec<u8>)> {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME { ctx } => PGPBackend::decrypt(ctx, cipher),
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
                            return Err(format!("{decrypt_command} exited with {output:?}").into());
                        }
                        if let Ok(err) = serde_json::from_slice::<String>(&output.stdout) {
                            return Err(err.into());
                        }
                        #[derive(Deserialize)]
                        struct Out {
                            recipients: Vec<Recipient>,
                            file_name: Option<String>,
                            session_key: Option<String>,
                            is_mime: bool,
                            data: Vec<u8>,
                        }
                        let out: Out = serde_json::from_slice(&output.stdout).map_err(|err| {
                            format!(
                                "Could not deserialize decryption response from \
                                 {decrypt_command}: {err}"
                            )
                        })?;
                        Ok((
                            DecryptionMetadata {
                                recipients: out.recipients,
                                file_name: out.file_name,
                                session_key: out.session_key,
                                is_mime: out.is_mime,
                            },
                            out.data,
                        ))
                    })
                    .await
                }))
            }
        }
    }
}
