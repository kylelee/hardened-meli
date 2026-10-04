//
// meli - accounts module.
//
// Copyright 2017 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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
// along with meli. If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

use indexmap::IndexMap;
use melib::{
    backends::{Mailbox, MailboxHash, MAX_MAILBOX_HIERARCHY_DEPTH},
    error::Error,
    tracing,
};
use smallvec::SmallVec;

use crate::{conf::FileMailboxConf, is_variant};

#[derive(Clone, Debug, Default)]
pub enum MailboxStatus {
    Available,
    Failed(Error),
    /// first argument is done work, and second is total work
    Parsing(usize, usize),
    #[default]
    None,
}

impl MailboxStatus {
    is_variant! { is_available, Available }
    is_variant! { is_parsing, Parsing(_, _) }
}

#[derive(Clone, Debug)]
pub struct MailboxEntry {
    pub status: MailboxStatus,
    pub name: String,
    pub path: String,
    pub ref_mailbox: Mailbox,
    pub conf: FileMailboxConf,
}

impl MailboxEntry {
    pub fn new(
        status: MailboxStatus,
        name: String,
        ref_mailbox: Mailbox,
        conf: FileMailboxConf,
    ) -> Self {
        let mut ret = Self {
            status,
            name,
            path: ref_mailbox.path().into(),
            ref_mailbox,
            conf,
        };
        match ret.conf.mailbox_conf.extra.get("encoding") {
            None => {}
            Some(v) if ["utf-8", "utf8"].iter().any(|e| v.eq_ignore_ascii_case(e)) => {}
            Some(v) if ["utf-7", "utf7"].iter().any(|e| v.eq_ignore_ascii_case(e)) => {
                ret.name = melib::backends::utf7::decode_utf7_imap(&ret.name);
                ret.path = melib::backends::utf7::decode_utf7_imap(&ret.path);
            }
            Some(other) => {
                tracing::warn!(
                    "mailbox `{}`: unrecognized mailbox name charset: {other}",
                    ret.name
                );
            }
        }
        ret
    }

    pub fn status(&self) -> String {
        match self.status {
            MailboxStatus::Available => format!(
                "{} [{} messages]",
                self.name(),
                self.ref_mailbox.count().ok().unwrap_or((0, 0)).1
            ),
            MailboxStatus::Failed(ref e) => e.to_string(),
            MailboxStatus::None => "Retrieving mailbox.".to_string(),
            MailboxStatus::Parsing(done, total) => {
                format!("Parsing messages. [{done}/{total}]")
            }
        }
    }

    pub fn name(&self) -> &str {
        if let Some(name) = self.conf.mailbox_conf.alias.as_ref() {
            name
        } else {
            self.ref_mailbox.name()
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct MailboxNode {
    pub hash: MailboxHash,
    pub depth: usize,
    pub indentation: u32,
    pub has_sibling: bool,
    pub children: Vec<Self>,
}

// CVE-2020-16094 (Claws Mail ≤ 3.17.6, CWE-674): a malicious IMAP server
// rebuilds an arbitrarily deep mailbox tree by stacking the hierarchy
// delimiter in LIST paths, and this owned tree is the exact equivalent of
// Claws Mail's directory tree. Every structural walk of it —
// construction in [`build_mailboxes_order`], the sidebar snapshot through
// `Account::list_mailboxes`, and teardown — used to spend one stack frame
// per hierarchy level (derive glue included), so a deep enough hostile
// tree walked the stack pointer into its guard page and aborted the
// process. `Clone` and `Drop` are therefore implemented iteratively: an
// explicit work stack spends constant stack no matter how deep the tree
// nests.
impl Clone for MailboxNode {
    fn clone(&self) -> Self {
        // Same two-pass arena discipline as the tree construction in
        // [`build_mailboxes_order`]: every node is copied into a flat
        // arena walked by an explicit stack, then linked into its parent
        // deepest-first (a child always has a higher arena index than its
        // parent, so reverse order moves every subtree intact). Children
        // are pushed onto the walk stack in *reverse* order: the LIFO pop
        // then creates them in natural sibling order, so their arena
        // indices ascend first-child-to-last and the deepest-first
        // linking — which pushes descending indices — restores the
        // original sibling order. The clone is structurally identical to
        // its source.
        let mut arena: Vec<Self> = Vec::new();
        let mut parent: Vec<Option<usize>> = Vec::new();
        let mut stack: Vec<(&Self, usize)> = Vec::new();
        let root_idx = arena.len();
        arena.push(Self {
            hash: self.hash,
            depth: self.depth,
            indentation: self.indentation,
            has_sibling: self.has_sibling,
            children: Vec::new(),
        });
        parent.push(None);
        stack.push((self, root_idx));
        while let Some((src, idx)) = stack.pop() {
            for sc in src.children.iter().rev() {
                let child = arena.len();
                arena.push(Self {
                    hash: sc.hash,
                    depth: sc.depth,
                    indentation: sc.indentation,
                    has_sibling: sc.has_sibling,
                    children: Vec::new(),
                });
                parent.push(Some(idx));
                stack.push((sc, child));
            }
        }
        let mut root = Self {
            hash: self.hash,
            depth: self.depth,
            indentation: self.indentation,
            has_sibling: self.has_sibling,
            children: Vec::new(),
        };
        for idx in (0..arena.len()).rev() {
            let node = std::mem::take(&mut arena[idx]);
            match parent[idx] {
                Some(p) => arena[p].children.push(node),
                None => root = node,
            }
        }
        root
    }
}

impl Drop for MailboxNode {
    fn drop(&mut self) {
        if self.children.is_empty() {
            return;
        }
        // Flatten the subtree onto a work list and let each node drop
        // with its children already moved out, so the compiler-generated
        // recursive drop glue never descends more than one level.
        let mut stack: Vec<Self> = std::mem::take(&mut self.children);
        while let Some(mut node) = stack.pop() {
            stack.append(&mut node.children);
        }
    }
}

pub fn build_mailboxes_order(
    tree: &mut Vec<MailboxNode>,
    mailbox_entries: &IndexMap<MailboxHash, MailboxEntry>,
    mailboxes_order: &mut Vec<MailboxHash>,
) {
    tree.clear();
    mailboxes_order.clear();
    for (h, f) in mailbox_entries.iter() {
        if f.ref_mailbox.parent().is_none() {
            // CVE-2020-16094 (Claws Mail ≤ 3.17.6, CWE-674): rebuild the
            // owned node tree iteratively, and never deeper than
            // `MAX_MAILBOX_HIERARCHY_DEPTH` levels. The recursive walk
            // this replaces spent one stack frame per hierarchy level, so
            // a mailbox tree nested deeper than the stack budget — a
            // malicious server's delimiter-stacked LIST paths, or any
            // other backend feeding arbitrary parent chains (the sync
            // cache included) — exhausted the stack exactly like Claws
            // Mail's directory-tree rebuild, and the unbounded subtree
            // clones of the sidebar snapshot exhausted memory with it.
            // Nodes are created in a flat arena walked by an explicit
            // stack, then linked into their parents deepest first (a
            // child always has a higher arena index than its parent, so
            // reverse order moves every subtree intact). Children are
            // pushed onto the walk stack in *reverse* order: the LIFO pop
            // then creates them in natural sibling order, so their arena
            // indices ascend first-child-to-last and the deepest-first
            // linking — which pushes descending indices — restores the
            // original sibling order. The depth cap keeps every later
            // walk of the tree (order, snapshot, teardown) linear in the
            // number of mailboxes, whatever the backend feeds it.
            let mut arena: Vec<MailboxNode> = Vec::new();
            let mut parent: Vec<Option<usize>> = Vec::new();
            let mut stack: SmallVec<[usize; 16]> = SmallVec::new();
            arena.push(MailboxNode {
                hash: *h,
                children: Vec::new(),
                depth: 0,
                indentation: 0,
                has_sibling: false,
            });
            parent.push(None);
            stack.push(0);
            let mut capped = false;
            while let Some(idx) = stack.pop() {
                // Mailboxes nested past the cap are not admitted to the
                // owned tree: their subtrees stay out of the sidebar and
                // out of every tree walk's budget.
                if arena[idx].depth + 1 >= MAX_MAILBOX_HIERARCHY_DEPTH {
                    capped = capped
                        || mailbox_entries[&arena[idx].hash]
                            .ref_mailbox
                            .children()
                            .iter()
                            .any(|&c| mailbox_entries.contains_key(&c));
                    continue;
                }
                for &c in mailbox_entries[&arena[idx].hash]
                    .ref_mailbox
                    .children()
                    .iter()
                    .rev()
                {
                    if mailbox_entries.contains_key(&c) {
                        let child = arena.len();
                        arena.push(MailboxNode {
                            hash: c,
                            children: Vec::new(),
                            depth: arena[idx].depth + 1,
                            indentation: 0,
                            has_sibling: false,
                        });
                        parent.push(Some(idx));
                        stack.push(child);
                    }
                }
            }
            if capped {
                tracing::warn!(
                    "mailbox hierarchy deeper than {} levels; deeper mailboxes not shown",
                    MAX_MAILBOX_HIERARCHY_DEPTH
                );
            }
            for idx in (0..arena.len()).rev() {
                let node = std::mem::take(&mut arena[idx]);
                match parent[idx] {
                    Some(p) => arena[p].children.push(node),
                    None => tree.push(node),
                }
            }
        }
    }

    macro_rules! mailbox_eq_key {
        ($mailbox:expr) => {{
            if let Some(sort_order) = $mailbox.conf.mailbox_conf.sort_order {
                (0, sort_order, $mailbox.ref_mailbox.path())
            } else {
                (1, 0, $mailbox.ref_mailbox.path())
            }
        }};
    }
    tree.sort_unstable_by(|a, b| {
        if mailbox_entries[&b.hash]
            .conf
            .mailbox_conf
            .sort_order
            .is_none()
            && mailbox_entries[&b.hash]
                .ref_mailbox
                .path()
                .eq_ignore_ascii_case("INBOX")
        {
            std::cmp::Ordering::Greater
        } else if mailbox_entries[&a.hash]
            .conf
            .mailbox_conf
            .sort_order
            .is_none()
            && mailbox_entries[&a.hash]
                .ref_mailbox
                .path()
                .eq_ignore_ascii_case("INBOX")
        {
            std::cmp::Ordering::Less
        } else {
            mailbox_eq_key!(mailbox_entries[&a.hash])
                .cmp(&mailbox_eq_key!(mailbox_entries[&b.hash]))
        }
    });

    let mut stack: SmallVec<[Option<&MailboxNode>; 16]> = SmallVec::new();
    for n in tree.iter_mut() {
        mailboxes_order.push(n.hash);
        n.children.sort_unstable_by(|a, b| {
            if mailbox_entries[&b.hash]
                .conf
                .mailbox_conf
                .sort_order
                .is_none()
                && mailbox_entries[&b.hash]
                    .ref_mailbox
                    .path()
                    .eq_ignore_ascii_case("INBOX")
            {
                std::cmp::Ordering::Greater
            } else if mailbox_entries[&a.hash]
                .conf
                .mailbox_conf
                .sort_order
                .is_none()
                && mailbox_entries[&a.hash]
                    .ref_mailbox
                    .path()
                    .eq_ignore_ascii_case("INBOX")
            {
                std::cmp::Ordering::Less
            } else {
                mailbox_eq_key!(mailbox_entries[&a.hash])
                    .cmp(&mailbox_eq_key!(mailbox_entries[&b.hash]))
            }
        });
        stack.extend(n.children.iter().rev().map(Some));
        while let Some(Some(next)) = stack.pop() {
            mailboxes_order.push(next.hash);
            stack.extend(next.children.iter().rev().map(Some));
        }
    }
    drop(stack);
    for node in tree.iter_mut() {
        // CVE-2020-16094: the indentation walk is flattened with the same
        // explicit-stack discipline as the construction above — constant
        // stack regardless of hierarchy depth. Semantics are preserved
        // exactly from the recursive form: only subscribed children
        // descend, the child indentation shifts one bit left per level
        // and inherits the parent's has_sibling bit, and a child's
        // has_sibling is "is not the last subscribed sibling". Children
        // are processed off the stack in reverse order, but each node's
        // (indentation, has_sibling) is computed from its parent alone,
        // so the final state is identical.
        let mut stack: SmallVec<[(&mut MailboxNode, u32, bool); 16]> = SmallVec::new();
        stack.push((node, 0, false));
        while let Some((node, indentation, has_sibling)) = stack.pop() {
            node.indentation = indentation;
            node.has_sibling = has_sibling;
            let subscribed = node
                .children
                .iter()
                .filter(|c| mailbox_entries[&c.hash].ref_mailbox.is_subscribed())
                .count();
            let mut child_indentation = indentation << 1;
            if has_sibling {
                child_indentation |= 1;
            }
            let mut seen = 0;
            for c in node.children.iter_mut() {
                if !mailbox_entries[&c.hash].ref_mailbox.is_subscribed() {
                    continue;
                }
                seen += 1;
                stack.push((c, child_indentation, seen != subscribed));
            }
        }
    }
}
