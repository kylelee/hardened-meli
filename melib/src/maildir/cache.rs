//
// melib
//
// Copyright 2026 Emmanouil Pitsidianakis <manos@pitsidianak.is>
// Copyright 2026 Kyle Lee
//
// This file is part of melib.
//
// melib is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// melib is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with melib. If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    backends::prelude::*,
    error::{IntoError, ResultIntoError},
    maildir::{
        Configuration, MaildirMailbox,
        utilities::{MaildirFilePathExt, MaildirMailboxPathExt},
    },
};

#[derive(Debug, Default)]
pub struct HashIndex {
    pub index: HashMap<EnvelopeHash, PathBuf>,
    pub reverse_index: HashMap<PathBuf, EnvelopeHash>,
    pub mailbox_hash: MailboxHash,
}

impl HashIndex {
    pub fn path_to_hash(&self, path: &Path) -> Option<EnvelopeHash> {
        self.reverse_index.get(path).cloned()
    }

    pub fn hash_to_path(&self, env_hash: &EnvelopeHash) -> Option<&PathBuf> {
        self.index.get(env_hash)
    }

    pub fn remove_env_hash(&mut self, env_hash: &EnvelopeHash) -> Option<PathBuf> {
        let path = self.index.remove(env_hash)?;
        self.reverse_index.remove(&path);
        Some(path)
    }
}

#[derive(Clone, Debug)]
pub struct Cache {
    pub account_name: Arc<String>,
    pub account_hash: AccountHash,
    pub mailboxes: Arc<Mutex<HashMap<MailboxHash, MaildirMailbox>>>,
    pub mailbox_index: Arc<Mutex<HashMap<EnvelopeHash, MailboxHash>>>,
    pub hash_indexes: Arc<Mutex<HashMap<MailboxHash, HashIndex>>>,
    pub buffer: Vec<u8>,
}

impl Cache {
    pub fn create(&mut self, path: &Path) -> Result<(MailboxHash, Envelope)> {
        let Some(mailbox_hash): Option<MailboxHash> = path.to_mailbox_hash() else {
            return Err(Error::new("Invalid mailbox path").set_kind(ErrorKind::ValueError));
        };
        // Take the `mailboxes` lock only long enough to clone the shared
        // counters, so that the potentially slow file I/O and parsing below
        // happen without holding any cache lock.
        let (total, unseen) = {
            let mailboxes_lck = self.mailboxes.lock().unwrap();
            let Some(mailbox) = mailboxes_lck.get(&mailbox_hash) else {
                return Err(
                    Error::new("Mailbox does not exist in cache").set_kind(ErrorKind::NotFound)
                );
            };
            (mailbox.total.clone(), mailbox.unseen.clone())
        };

        let env_hash = path.to_envelope_hash();
        let mut reader =
            std::io::BufReader::new(std::fs::File::open(path).chain_err_related_path(path)?);
        self.buffer.clear();
        reader
            .read_to_end(&mut self.buffer)
            .chain_err_related_path(path)?;

        let mut env = Envelope::from_bytes(self.buffer.as_slice(), Some(path.flags()))
            .chain_err_related_path(path)?;
        env.set_hash(env_hash);

        let mut hi_lck = self.hash_indexes.lock().unwrap();
        let mut mi = self.mailbox_index.lock().unwrap();
        let hi = hi_lck.entry(mailbox_hash).or_default();
        hi.index.insert(env_hash, path.to_path_buf());
        hi.reverse_index.insert(path.to_path_buf(), env_hash);
        mi.insert(env.hash(), mailbox_hash);
        total.lock().unwrap().insert_new(env_hash);
        if !env.is_seen() {
            unseen.lock().unwrap().insert_new(env_hash);
        }
        Ok((mailbox_hash, env))
    }

    pub fn remove(&self, path: &Path) -> Option<(MailboxHash, EnvelopeHash)> {
        let mut hi = self.hash_indexes.lock().unwrap();
        let mut mi = self.mailbox_index.lock().unwrap();
        let mailboxes_lck = self.mailboxes.lock().unwrap();
        let (mailbox_hash, env_hash) = hi
            .iter()
            .find_map(|(k, v)| v.path_to_hash(path).map(|v| (*k, v)))?;
        let mailbox = mailboxes_lck.get(&mailbox_hash)?;
        mi.remove(&env_hash);
        hi.entry(mailbox_hash)
            .or_default()
            .remove_env_hash(&env_hash);
        // Removing a hash that is not counted any more is a no-op, so a
        // duplicate removal cannot underflow the counts.
        mailbox.total.lock().unwrap().remove(env_hash);
        mailbox.unseen.lock().unwrap().remove(env_hash);
        Some((mailbox_hash, env_hash))
    }

    pub fn remove_env_hash(&self, env_hash: EnvelopeHash) -> bool {
        let mut hi = self.hash_indexes.lock().unwrap();
        let mut mi = self.mailbox_index.lock().unwrap();
        let mailboxes_lck = self.mailboxes.lock().unwrap();
        let Some(mailbox_hash) = mi.remove(&env_hash) else {
            return false;
        };
        let Some(mailbox) = mailboxes_lck.get(&mailbox_hash) else {
            return false;
        };
        hi.entry(mailbox_hash)
            .or_default()
            .remove_env_hash(&env_hash);
        // Removing a hash that is not counted any more is a no-op, so a
        // duplicate removal cannot underflow the counts.
        mailbox.total.lock().unwrap().remove(env_hash);
        mailbox.unseen.lock().unwrap().remove(env_hash);
        true
    }

    /// Delete a file and forget its cache entries, so that user-initiated
    /// deletions do not have to wait for a refresh to be reflected.
    pub fn remove_env_and_file(&self, env_hash: EnvelopeHash) -> Result<()> {
        let path = self.hash_to_path(&env_hash);
        if let Some(path) = path {
            if let Err(err) = std::fs::remove_file(&path) {
                // The file may already be gone (e.g. renamed concurrently by
                // another process); only a real failure is an error.
                if err.kind() != std::io::ErrorKind::NotFound {
                    return Err(Error::from(err).set_err_related_path(&path));
                }
            }
        }
        self.remove_env_hash(env_hash);
        Ok(())
    }

    pub fn path_to_hash(&self, path: &Path) -> Option<(MailboxHash, EnvelopeHash)> {
        self.hash_indexes
            .lock()
            .unwrap()
            .iter()
            .find_map(|(k, v)| v.path_to_hash(path).map(|v| (*k, v)))
    }

    pub fn hash_to_path(&self, env_hash: &EnvelopeHash) -> Option<PathBuf> {
        self.hash_indexes
            .lock()
            .unwrap()
            .values()
            .find_map(|v| v.hash_to_path(env_hash).cloned())
    }

    /// Rename a maildir file with new flags and update the cache to match,
    /// returning the new flags and hash, or `None` when there is nothing to
    /// do (envelope unknown to this mailbox, or flags did not change).
    ///
    /// Filesystem I/O happens without holding any cache lock: the path and
    /// flags are resolved under the index lock first, the rename runs
    /// lock-free, and the result is published back afterwards.
    pub fn set_flags(
        &self,
        env_hash: EnvelopeHash,
        mailbox_hash: MailboxHash,
        flag_ops: &[FlagOp],
        config: &Configuration,
    ) -> Result<Option<(Flag, EnvelopeHash)>> {
        let (path, new_flags) = {
            let hash_indexes_lck = self.hash_indexes.lock().unwrap();
            let Some(hash_index) = hash_indexes_lck.get(&mailbox_hash) else {
                return Err(
                    Error::new("Mailbox does not exist in cache").set_kind(ErrorKind::NotFound)
                );
            };
            let Some(path) = hash_index.index.get(&env_hash) else {
                return Ok(None);
            };
            let old_flags = path.flags();
            let mut new_flags = old_flags;
            for op in flag_ops {
                if let FlagOp::Set(f) | FlagOp::UnSet(f) = op {
                    new_flags.set(*f, op.as_bool());
                }
            }
            if new_flags == old_flags {
                return Ok(None);
            }
            (path.clone(), new_flags)
        };

        let new_name: PathBuf = path.set_flags(new_flags, config)?;
        debug_assert_eq!(new_flags, new_name.flags());
        log::trace!("renaming {path:?} to {new_name:?}");
        std::fs::rename(&path, &new_name).chain_err_summary(|| {
            format!("Could not rename {path:?} to {new_name:?} when setting maildir flags")
        })?;
        log::trace!("success in rename");

        let mut hash_indexes_lck = self.hash_indexes.lock().unwrap();
        let mut mi = self.mailbox_index.lock().unwrap();
        let mailboxes_lck = self.mailboxes.lock().unwrap();
        let Some(mailbox) = mailboxes_lck.get(&mailbox_hash) else {
            return Err(Error::new("Mailbox does not exist in cache").set_kind(ErrorKind::NotFound));
        };
        let hash_index = hash_indexes_lck.entry(mailbox_hash).or_default();
        if hash_index.index.get(&env_hash) != Some(&path) {
            // The index moved on while we were renaming; leave the cache as
            // it is and let the notify watcher reconcile it.
            return Ok(None);
        }
        let new_hash = new_name.to_envelope_hash();
        hash_index.index.insert(new_hash, new_name.clone());
        hash_index.reverse_index.insert(new_name, new_hash);
        hash_index.remove_env_hash(&env_hash);
        mi.remove(&env_hash);
        mi.insert(new_hash, mailbox_hash);
        mailbox.total.lock().unwrap().remove(env_hash);
        mailbox.total.lock().unwrap().insert_new(new_hash);
        mailbox.unseen.lock().unwrap().remove(env_hash);
        if !new_flags.contains(Flag::SEEN) {
            mailbox.unseen.lock().unwrap().insert_new(new_hash);
        }
        Ok(Some((new_flags, new_hash)))
    }
}
