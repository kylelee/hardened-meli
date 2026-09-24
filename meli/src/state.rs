/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
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

//! The application's state.
//!
//! The UI crate has an [`Box<dyn
//! Component>`](crate::components::Component)-Component-System design. The
//! system part, is also the application's state, so they're both merged in the
//! [`State`] struct.
//!
//! [`State`] owns all the Components of the UI. In the application's main event
//! loop, input is handed to the state in the form of [`UIEvent`] objects which
//! traverse the component graph. Components decide to handle each input or not.
//!
//! Input is received in the main loop from threads which listen on the stdin
//! for user input, observe folders for file changes etc. The relevant struct is
//! [`ThreadEvent`].

use std::{
    borrow::Cow,
    collections::BTreeSet,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use crossbeam::channel::{unbounded, Receiver, Sender};
use indexmap::{IndexMap, IndexSet};
use melib::{
    backends::{
        AccountHash, BackendEvent, BackendEventConsumer, Backends, RefreshEvent, RefreshEventKind,
    },
    utils::datetime,
};
use smallvec::SmallVec;

use super::*;

/// Debug-build tracing span for draw paths: logs `begin` on construction
/// and `done in <elapsed>` on drop, so *any* exit path (early return,
/// `?`, panic unwinding) leaves its trail. A freeze report's last `begin`
/// without its `done` names the exact draw that never returned.
///
/// Compiles to nothing in release builds (logging is compiled out there
/// anyway, and the guard is only constructed from debug code paths).
#[cfg(debug_assertions)]
pub(crate) struct DrawSpan {
    name: String,
    start: std::time::Instant,
}

#[cfg(debug_assertions)]
impl DrawSpan {
    pub(crate) fn enter(name: &str) -> Self {
        log::debug!("draw: {name} begin");
        Self {
            name: name.to_string(),
            start: std::time::Instant::now(),
        }
    }
}

#[cfg(debug_assertions)]
impl Drop for DrawSpan {
    fn drop(&mut self) {
        log::debug!("draw: {} done in {:?}", self.name, self.start.elapsed());
    }
}
use crate::{
    conf::data_types::SearchBackend,
    jobs::JobExecutor,
    notifications::DisplayMessageBox,
    terminal::{get_events, Screen, Tty},
};

struct InputHandler {
    pipe: (OwnedFd, OwnedFd),
    rx: Receiver<InputCommand>,
    tx: Sender<InputCommand>,
    state_tx: Sender<ThreadEvent>,
    control: std::sync::Weak<()>,
}

impl InputHandler {
    fn restore(&mut self) {
        let working = Arc::new(());
        let control = Arc::downgrade(&working);

        /* Clear channel without blocking. switch_to_main_screen() issues a kill when
         * returning from a fork and there's no input thread, so the newly created
         * thread will receive it and die. */
        //let _ = self.rx.try_iter().count();
        let rx = self.rx.clone();
        let pipe = nix::unistd::dup(&self.pipe.0)
            .expect("Fatal: Could not dup() input pipe file descriptor");
        let tx = self.state_tx.clone();
        let resize_tx = self.state_tx.clone();
        thread::Builder::new()
            .name("input-thread".to_string())
            .spawn(move || loop {
                // A panic anywhere in the parse/delivery chain (crossterm,
                // the fd swap guards, the send callbacks) must not kill the
                // thread: with it gone, no input ever reaches the main loop
                // again and the UI looks frozen to the user. The fd swap
                // guards restore their descriptions during unwinding, so a
                // restart re-enters `get_events` on a clean state. If the
                // panic is deterministic, back off instead of spinning.
                let run = std::panic::AssertUnwindSafe(|| {
                    let working = working.clone();
                    get_events(
                        |i| {
                            tx.send(ThreadEvent::Input(i)).unwrap();
                        },
                        |cols, rows| {
                            log::trace!("terminal resized to {cols}x{rows}");
                            resize_tx
                                .send(ThreadEvent::UIEvent(UIEvent::Resize))
                                .unwrap();
                        },
                        &rx,
                        &pipe,
                        working,
                    )
                });
                if std::panic::catch_unwind(run).is_err() {
                    log::error!("input thread panicked; restarting it");
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                }
                // `get_events` returns only on a kill command.
                break;
            })
            .unwrap();
        self.control = control;
    }

    fn kill(&self) {
        let _ = nix::unistd::write(&self.pipe.1, &[1]);
        self.tx.send(InputCommand::Kill).unwrap();
    }

    fn check(&mut self) {
        match self.control.upgrade() {
            Some(_) => {}
            None => {
                log::trace!("restarting input_thread");
                self.restore();
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct MainLoopHandler {
    pub sender: Sender<ThreadEvent>,
    pub job_executor: Arc<JobExecutor>,
}

impl MainLoopHandler {
    #[inline]
    pub fn send(&self, event: ThreadEvent) {
        if let Err(err) = self.sender.send(event) {
            log::error!("Could not send event to main loop: {}", err);
        }
    }
}

/// A context container for loaded settings, accounts, UI changes, etc.
pub struct Context {
    pub accounts: IndexMap<AccountHash, Account>,
    pub settings: Box<Settings>,

    /// Areas of the screen that must be redrawn in the next render
    pub dirty_areas: VecDeque<Area>,

    /// Events queue that components send back to the state
    pub replies: VecDeque<UIEvent>,
    pub realized: IndexMap<ComponentId, Option<ComponentId>>,
    pub unrealized: IndexSet<ComponentId>,
    pub main_loop_handler: MainLoopHandler,
    pub receiver: Receiver<ThreadEvent>,
    input_thread: InputHandler,
    current_dir: PathBuf,
    /// The `cmd_buf` is a integer buffer that accumulates pressed digit
    /// buttons.
    ///
    /// When a movement command is received, such as "move cursor" down, a
    /// component might choose to multiply that movement with the value in
    /// the buffer.
    ///
    /// Since pushing/popping/clearing operations need to also update the status
    /// buffer on the bottom right, this field is only accessible through
    /// special [`Context`] methods.
    cmd_buf: Option<usize>,
    /// Children processes
    pub children: IndexMap<Cow<'static, str>, Vec<ForkedProcess>>,
    pub temp_files: Vec<File>,
}

impl Context {
    pub fn replies(&mut self) -> smallvec::SmallVec<[UIEvent; 8]> {
        self.replies.drain(0..).collect()
    }

    pub fn input_kill(&self) {
        self.input_thread.kill();
    }

    pub fn restore_input(&mut self) {
        self.input_thread.restore();
    }

    pub fn is_online_idx(&mut self, account_pos: usize) -> Result<()> {
        let Self {
            ref mut accounts,
            ref mut replies,
            ..
        } = self;
        let was_online = accounts[account_pos].is_online.is_true();
        let ret = accounts[account_pos].is_online(false);
        if ret.is_ok() && !was_online {
            log::trace!("inserting mailbox hashes:");
            for mailbox_node in accounts[account_pos].list_mailboxes() {
                log::trace!(
                    "hash & mailbox: {:?} {}",
                    mailbox_node.hash,
                    accounts[account_pos][&mailbox_node.hash].name()
                );
            }
            accounts[account_pos].watch(None);

            replies.push_back(UIEvent::AccountStatusChange(
                accounts[account_pos].hash(),
                None,
            ));
        }
        if ret.is_ok() != was_online {
            replies.push_back(UIEvent::AccountStatusChange(
                accounts[account_pos].hash(),
                None,
            ));
        }
        ret
    }

    pub fn is_online(&mut self, account_hash: AccountHash) -> Result<()> {
        let idx = self
            .accounts
            .get_index_of(&account_hash)
            .ok_or_else(|| Error::new("Unknown account.").set_kind(ErrorKind::Configuration))?;
        self.is_online_idx(idx)
    }

    #[cfg(test)]
    pub fn new_mock(dir: &tempfile::TempDir) -> Self {
        use crate::conf::tests::{ConfigFile, IMAP_CONFIG};

        let (sender, receiver) =
            crossbeam::channel::bounded(32 * ::std::mem::size_of::<ThreadEvent>());
        let job_executor = Arc::new(JobExecutor::new(sender.clone()));
        let input_thread = unbounded();
        let input_thread_pipe = crate::types::pipe().unwrap();
        let backends = Backends::new();
        let config_file = ConfigFile::new(IMAP_CONFIG, dir).unwrap();
        let mut settings = Box::new(Settings::from_path(config_file.path.clone()).unwrap());
        // Pin the color decision: `TerminalSettings::use_color()` also
        // consults the `NO_COLOR` environment variable, so a developer shell
        // with it set (or unset) would drift golden baselines and any other
        // test that renders UI attributes. An explicit `false` matches the
        // recorded corpus (its `Attr::REVERSE` fallback accents are part of
        // the pinned frames) and is independent of the environment.
        settings.terminal.use_color = melib::ToggleFlag::False;
        let accounts = vec![{
            let name = "test".to_string();
            let mut account_conf = crate::conf::AccountConf::default();
            account_conf.conf.format = "maildir".to_string();
            account_conf.account.format = "maildir".to_string();
            account_conf.account.root_mailbox = dir.path().display().to_string();
            let sender = sender.clone();
            let account_hash = AccountHash::from_bytes(name.as_bytes());
            Account::new(
                account_hash,
                name,
                account_conf,
                &backends,
                MainLoopHandler {
                    job_executor: job_executor.clone(),
                    sender: sender.clone(),
                },
                BackendEventConsumer::new(Arc::new(
                    move |account_hash: AccountHash, ev: BackendEvent| {
                        sender
                            .send(ThreadEvent::UIEvent(UIEvent::BackendEvent(
                                account_hash,
                                ev,
                            )))
                            .unwrap();
                    },
                )),
            )
            .unwrap()
        }];
        let accounts = accounts.into_iter().map(|acc| (acc.hash(), acc)).collect();
        let working = Arc::new(());
        let control = Arc::downgrade(&working);
        Self {
            accounts,
            settings,
            dirty_areas: VecDeque::with_capacity(0),
            replies: VecDeque::with_capacity(0),
            realized: IndexMap::default(),
            unrealized: IndexSet::default(),
            temp_files: Vec::new(),
            current_dir: std::env::current_dir().unwrap(),
            children: IndexMap::default(),
            cmd_buf: None,

            input_thread: InputHandler {
                pipe: input_thread_pipe,
                rx: input_thread.1,
                tx: input_thread.0,
                control,
                state_tx: sender.clone(),
            },
            main_loop_handler: MainLoopHandler {
                job_executor,
                sender,
            },
            receiver,
        }
    }

    #[inline]
    pub fn cmd_buf(&self) -> Option<usize> {
        self.cmd_buf
    }

    #[inline]
    pub fn cmd_buf_push(&mut self, c: char, modifier_command: Option<Modifier>) {
        if !c.is_ascii_digit() {
            return;
        }
        let Some(mut cmd_buf) = self.cmd_buf.unwrap_or(0).checked_mul(10) else {
            return;
        };
        cmd_buf += (c as u32 - '0' as u32) as usize;
        self.replies
            .push_back(UIEvent::StatusEvent(StatusEvent::BufSet(
                if let Some(modf) = modifier_command {
                    format!("{modf} {cmd_buf}")
                } else {
                    cmd_buf.to_string()
                },
            )));
        self.cmd_buf = Some(cmd_buf);
    }

    #[inline]
    pub fn cmd_buf_pop(&mut self, modifier_command: Option<Modifier>) {
        if self.cmd_buf.is_none() {
            return;
        }
        let mut cmd_buf = self.cmd_buf.unwrap_or(0);
        cmd_buf /= 10;
        if cmd_buf == 0 {
            self.cmd_buf = None;
            self.replies
                .push_back(UIEvent::StatusEvent(StatusEvent::BufClear));
            return;
        }
        self.replies
            .push_back(UIEvent::StatusEvent(StatusEvent::BufSet(
                if let Some(modf) = modifier_command {
                    format!("{modf} {cmd_buf}")
                } else {
                    cmd_buf.to_string()
                },
            )));
        self.cmd_buf = Some(cmd_buf);
    }

    #[inline]
    #[must_use]
    pub fn cmd_buf_clear(&mut self) -> Option<usize> {
        if self.cmd_buf.is_some() {
            self.replies
                .push_back(UIEvent::StatusEvent(StatusEvent::BufClear));
        }
        std::mem::take(&mut self.cmd_buf)
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }
}

/// A State object to manage and own components and components of the UI.
/// `State` is responsible for managing the terminal and interfacing with
/// `melib`
pub struct State {
    screen: Box<Screen<Tty>>,
    draw_rate_limit: RateLimit,
    child: Option<ForkedProcess>,
    pub mode: UIMode,
    /// Set when a `UIEvent::Exit` request arrives; the main loop checks
    /// it after draining replies and performs the actual shutdown.
    pub exit_requested: bool,
    overlay: IndexMap<ComponentId, Box<dyn Component>>,
    components: IndexMap<ComponentId, Box<dyn Component>>,
    component_tree: IndexMap<ComponentId, ComponentPath>,
    pub context: Box<Context>,
    timer: thread::JoinHandle<()>,
    message_box: DisplayMessageBox,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(Err(err)) = self.kill_main_child() {
            log::debug!("Failed to kill subprocess: {}", err);
        }
        if let (Some(false), Some(child)) = (self.try_wait_on_main_child(), self.child.as_ref()) {
            log::error!("Main subprocess {:?} is still running on exit!", child);
        }
        let mut other_children = std::mem::take(&mut self.context.children);
        for (id, child, err) in other_children
            .iter_mut()
            .flat_map(|(i, v)| v.iter_mut().map(move |v| (i, v)))
            .filter_map(|(id, child)| {
                if let Err(err) = child.kill() {
                    Some((id, child, err))
                } else {
                    None
                }
            })
        {
            log::error!("Failed to kill subprocess {} ({:?}): {}", id, child, err);
        }
        for (id, child, err) in other_children
            .into_iter()
            .map(|(i, v)| (std::rc::Rc::new(i), v))
            .flat_map(|(i, v)| v.into_iter().map(move |v| (i.clone(), v)))
            .filter_map(|(id, mut child)| {
                if let Err(err) = child.try_wait() {
                    Some((id, child, err))
                } else {
                    None
                }
            })
        {
            log::error!(
                "Failed to wait for subprocess {} ({:?}): {}",
                id,
                child,
                err
            );
        }
        // When done, restore the defaults to avoid messing with the terminal.
        self.screen.switch_to_main_screen();
    }
}

impl State {
    pub fn new(
        settings: Option<Settings>,
        sender: Sender<ThreadEvent>,
        receiver: Receiver<ThreadEvent>,
    ) -> Result<Self> {
        // Create async channel to block the input-thread if we need to fork and stop it
        // from reading stdin, see get_events() for details
        let input_thread = unbounded();
        let input_thread_pipe = crate::types::pipe()?;
        let backends = Backends::new();
        let settings = Box::new(if let Some(settings) = settings {
            settings
        } else {
            Settings::new()?
        });

        // A configuration without accounts cannot start: the mail listing
        // needs an account to point its offline fallback at, and indexing
        // an empty account map used to panic on startup. Fail with a clear
        // configuration error instead.
        if settings.accounts.is_empty() {
            return Err(Error::new(
                "No accounts are configured. Add at least one `[accounts.<name>]` section to your \
                 configuration file; see the `accounts` section of meli.conf(5).",
            )
            .set_kind(ErrorKind::Configuration));
        }

        let (cols, rows) = crossterm::terminal::size().chain_err_summary(|| {
            "Could not determine terminal size. Are you running this on a tty? If yes, do you need \
             permissions for tty ioctls?"
        })?;
        let (cols, rows) = (cols as usize, rows as usize);

        let job_executor = Arc::new(JobExecutor::new(sender.clone()));
        let accounts = {
            settings
                .accounts
                .iter()
                .map(|(n, a_s)| {
                    let sender = sender.clone();
                    let account_hash = AccountHash::from_bytes(n.as_bytes());
                    Account::new(
                        account_hash,
                        n.to_string(),
                        a_s.clone(),
                        &backends,
                        MainLoopHandler {
                            job_executor: job_executor.clone(),
                            sender: sender.clone(),
                        },
                        BackendEventConsumer::new(Arc::new(
                            move |account_hash: AccountHash, ev: BackendEvent| {
                                sender
                                    .send(ThreadEvent::UIEvent(UIEvent::BackendEvent(
                                        account_hash,
                                        ev,
                                    )))
                                    .unwrap();
                            },
                        )),
                    )
                })
                .collect::<Result<Vec<Account>>>()?
        };
        let accounts = accounts.into_iter().map(|acc| (acc.hash(), acc)).collect();

        let timer = {
            let sender = sender.clone();
            thread::Builder::new().spawn(move || {
                let sender = sender;
                loop {
                    thread::park();

                    sender.send(ThreadEvent::Pulse).unwrap();
                    thread::sleep(std::time::Duration::from_millis(100));
                }
            })
        }?;

        timer.thread().unpark();

        let working = Arc::new(());
        let control = Arc::downgrade(&working);
        let mut screen =
            Box::new(Screen::<Tty>::new(Default::default()).with_cols_and_rows(cols, rows));
        screen
            .tty_mut()
            .set_mouse(settings.terminal.use_mouse.is_true())
            .set_draw_fn(if settings.terminal.use_color() {
                Screen::draw_horizontal_segment
            } else {
                Screen::draw_horizontal_segment_no_color
            });
        let message_box = DisplayMessageBox::new(&screen);
        let mut s = Self {
            screen,
            child: None,
            mode: UIMode::Normal,
            exit_requested: false,
            components: IndexMap::default(),
            overlay: IndexMap::default(),
            component_tree: IndexMap::default(),
            timer,
            draw_rate_limit: RateLimit::new(1, 3, job_executor.clone()),
            message_box,
            context: Box::new(Context {
                accounts,
                settings,
                dirty_areas: VecDeque::with_capacity(5),
                replies: VecDeque::with_capacity(5),
                realized: IndexMap::default(),
                unrealized: IndexSet::default(),
                temp_files: Vec::new(),
                current_dir: std::env::current_dir()?,
                children: IndexMap::default(),
                cmd_buf: None,
                input_thread: InputHandler {
                    pipe: input_thread_pipe,
                    rx: input_thread.1,
                    tx: input_thread.0,
                    control,
                    state_tx: sender.clone(),
                },
                main_loop_handler: MainLoopHandler {
                    job_executor,
                    sender,
                },
                receiver,
            }),
        };
        if s.context.settings.terminal.ascii_drawing {
            s.screen.grid_mut().set_ascii_drawing(true);
            s.screen.overlay_grid_mut().set_ascii_drawing(true);
        }
        if s.context.settings.terminal.use_text_presentation() {
            s.screen.grid_mut().set_force_text_presentation(true);
            s.screen
                .overlay_grid_mut()
                .set_force_text_presentation(true);
        }
        if s.context.settings.terminal.draw_hyperlinks() {
            s.screen.grid_mut().set_draw_hyperlinks(true);
            s.screen.overlay_grid_mut().set_draw_hyperlinks(true);
        }

        s.screen.switch_to_alternate_screen(&s.context);
        s.screen.do_background_query();
        for i in 0..s.context.accounts.len() {
            if !s.context.accounts[i].backend_capabilities.is_remote {
                s.context.accounts[i].watch(None);
            }
            if s.context.is_online_idx(i).is_ok() && s.context.accounts[i].is_empty() {
                //return Err(Error::new(format!(
                //    "Account {} has no mailboxes configured.",
                //    s.context.accounts[i].name()
                //)));
            }
        }
        s.context.restore_input();
        // Non-fatal configuration problems were logged at load time; surface
        // them to the user as well instead of silently defaulting.
        let config_warnings = s.context.settings.config_warnings.clone();
        for warning in config_warnings {
            s.context.replies.push_back(UIEvent::Notification {
                title: Some("Configuration warning".into()),
                body: warning.into(),
                source: None,
                kind: Some(NotificationType::Error(ErrorKind::Configuration)),
            });
        }
        Ok(s)
    }

    /*
     * When we receive a mailbox hash from a watcher thread,
     * we match the hash to the index of the mailbox, request a reload
     * and startup a thread to remind us to poll it every now and then till it's
     * finished.
     */
    pub fn refresh_event(
        &mut self,
        account_hash: AccountHash,
        mailbox_hash: MailboxHash,
        events: Vec<RefreshEventKind>,
    ) {
        // `account_hash` and `mailbox_hash` originate in backend refresh
        // events; an unknown hash must be ignored, not indexed into the maps.
        let known_mailbox = self
            .context
            .accounts
            .get(&account_hash)
            .is_some_and(|acc| acc.mailbox_entries.contains_key(&mailbox_hash));
        if !known_mailbox {
            return;
        }
        if self
            .context
            .accounts
            .get_mut(&account_hash)
            .is_some_and(|acc| acc.load(mailbox_hash, false).is_err())
        {
            if let Some(acc) = self.context.accounts.get_mut(&account_hash) {
                acc.event_queue
                    .entry(mailbox_hash)
                    .or_default()
                    .extend(events);
            }
            return;
        }
        let notifications = self
            .context
            .accounts
            .get_mut(&account_hash)
            .and_then(|acc| acc.consume_refresh_events(events, mailbox_hash));

        if let Some(notifications) = notifications {
            for n in notifications {
                if matches!(n, UIEvent::Notification { .. }) {
                    self.rcv_event(UIEvent::MailboxUpdate((account_hash, mailbox_hash)));
                }
                self.rcv_event(n);
            }
        }
    }

    pub fn receiver(&self) -> Receiver<ThreadEvent> {
        self.context.receiver.clone()
    }

    pub fn sender(&self) -> Sender<ThreadEvent> {
        self.context.main_loop_handler.sender.clone()
    }

    pub fn restore_input(&mut self) {
        self.context.restore_input();
    }

    /// On `SIGWINCH` the `State` redraws itself according to the new terminal
    /// size.
    pub fn update_size(&mut self) {
        self.screen.update_size();
        self.rcv_event(UIEvent::Resize);
        self.message_box.set_dirty(true);
        self.message_box.initialised = false;

        // Invalidate dirty areas.
        self.context.dirty_areas.clear();
    }

    /// Build and overlay the theme picker dialog (`:toggle_theme`).
    ///
    /// The picker lists the themes compiled into the binary (the Zed
    /// editor's One/Ayu/Gruvbox family and the community ports, see
    /// [`builtin_themes`](crate::conf::builtin_themes)), every
    /// additional theme declared in the user's theme directory
    /// (`$XDG_CONFIG_HOME/meli/themes/*.toml`, tables
    /// `[terminal.themes.<name>]`) and every theme already embedded in the
    /// configuration. Arrow keys live-preview each entry (the preview is
    /// applied immediately, see [`State::apply_theme`]); `Enter` persists
    /// the highlighted theme to the configuration file; the quit binding
    /// (`q`/`Esc` by default) closes the picker without any theme action -
    /// the last previewed theme simply stays applied in memory.
    fn open_theme_picker(&mut self) {
        use crate::utilities::UIDialog;

        let (user_themes, errors) = crate::conf::get_user_themes();
        for err in errors {
            self.context.replies.push_back(UIEvent::Notification {
                title: Some("theme directory".into()),
                source: None,
                body: err.into(),
                kind: Some(NotificationType::Info),
            });
        }

        // Each name appears once, labeled with its highest-priority
        // source: a theme directory file shadows a configuration table,
        // which shadows the compiled-in theme.
        let names =
            crate::conf::theme_picker_entries(&self.context.settings.terminal.themes, &user_themes);

        let current = self.context.settings.terminal.theme.clone();
        let mut dialog: crate::utilities::UIDialog<String> = UIDialog::new(
            "theme",
            names,
            /* single_only */ true,
            /* done_fn */ None,
            &self.context,
        );
        dialog.set_cursor_to(&current);
        dialog.set_cursor_callback(Some(Box::new(|name: &String, context: &mut Context| {
            // Handled in `rcv_event`: applies the theme in-memory for
            // an immediate live preview of the whole UI.
            context.replies.push_back(UIEvent::ChangeTheme {
                name: name.clone(),
                persist: false,
            });
        })));
        // `Enter` finalises with the highlighted theme; the quit binding
        // (`q`/`Esc`) only closes the picker - theme selection takes
        // effect exclusively through the arrow-key live preview above and
        // `Enter`'s persist. Closing therefore leaves the last previewed
        // theme applied in memory, without touching the configuration.
        let restore = current;
        dialog.set_done_fn(Some(Box::new(
            move |_id, selection: &[String]| match selection.first() {
                Some(name) => Some(crate::types::UIEvent::ChangeTheme {
                    name: name.clone(),
                    persist: true,
                }),
                None => Some(crate::types::UIEvent::ChangeTheme {
                    name: restore,
                    persist: false,
                }),
            },
        )));
        let id = dialog.id();
        dialog.realize(None, &mut self.context);
        self.overlay.insert(id, Box::new(dialog));
        self.rcv_event(UIEvent::Resize);
    }

    /// Switch `settings.terminal.theme` to `name` and refresh every
    /// component (`UIEvent::ConfigReload`). When `name` names a theme from
    /// the user's theme directory, its definition is re-read from the file
    /// on every apply, so edits to it take effect between switches. With
    /// `persist`, also rewrite the `theme` value in the configuration file.
    fn apply_theme(&mut self, name: String, persist: bool) {
        let result = self.load_theme_into_settings(&name).and_then(|()| {
            if persist {
                self.persist_theme_to_config(&name)
            } else {
                Ok(())
            }
        });
        match result {
            Ok(()) => {
                self.context.settings.terminal.theme = name;
                let old_settings = self.context.settings.clone();
                self.context
                    .replies
                    .push_back(UIEvent::ConfigReload { old_settings });
                // No Resize here: ConfigReload already refreshes every
                // component's colors; a Resize forces an extra full-screen
                // pass per preview step, which the theme picker triggers on
                // every arrow key - the visible effect is flickering.
                self.context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::UpdateSubStatus(
                        String::new(),
                    )));
            }
            Err(err) => {
                self.context.replies.push_back(UIEvent::Notification {
                    title: Some("Could not switch theme".into()),
                    source: None,
                    body: err.to_string().into(),
                    kind: Some(NotificationType::Error(err.kind)),
                });
            }
        }
    }

    /// Ensure the theme `name` is present in
    /// `settings.terminal.themes`: built-ins and configuration themes
    /// already are; a theme from the user's theme directory is parsed
    /// from its file and inserted over any same-name definition, since a
    /// directory file shadows both built-ins and configuration tables
    /// (theme directory > configuration > built-in).
    fn load_theme_into_settings(&mut self, name: &str) -> melib::Result<()> {
        if name == crate::conf::LIGHT || name == crate::conf::DARK {
            return Ok(());
        }
        let (user_themes, _errors) = crate::conf::get_user_themes();
        if let Some(path) = user_themes.get(name) {
            // Re-reading the file on every apply keeps edits to it
            // effective between switches; applies are user-paced and the
            // parse cost is negligible next to the `get_user_themes()`
            // scan above.
            let theme = crate::conf::theme_from_file(
                name,
                path,
                &self.context.settings.terminal.themes.dark,
            )?;
            self.context
                .settings
                .terminal
                .themes
                .other_themes
                .insert(name.to_string(), theme);
            return Ok(());
        }
        if self
            .context
            .settings
            .terminal
            .themes
            .other_themes
            .contains_key(name)
        {
            return Ok(());
        }
        Err(melib::Error::new(format!(
            "theme `{name}` is not defined in the configuration or the themes directory"
        )))
    }

    /// Rewrite `theme = "<name>"` inside the `[terminal]` table of the
    /// configuration file, preserving every other value verbatim. When no
    /// `[terminal]` table exists, one is appended. The write is atomic
    /// (temp file + rename) so a crash cannot truncate the configuration.
    fn persist_theme_to_config(&mut self, name: &str) -> melib::Result<()> {
        let config_path = crate::conf::get_config_file()?;
        let text = std::fs::read_to_string(&config_path).map_err(|err| {
            melib::Error::new(format!("could not read {}: {err}", config_path.display()))
        })?;
        let updated = crate::conf::rewrite_terminal_theme(&text, name)?;
        let tmp = config_path.with_extension("toml.tmp");
        std::fs::write(&tmp, updated).map_err(|err| {
            melib::Error::new(format!("could not write {}: {err}", tmp.display()))
        })?;
        std::fs::rename(&tmp, &config_path).map_err(|err| {
            melib::Error::new(format!(
                "could not replace {}: {err}",
                config_path.display()
            ))
        })?;
        self.context
            .replies
            .push_back(UIEvent::StatusEvent(StatusEvent::UpdateStatus(format!(
                "theme saved: {name}"
            ))));
        Ok(())
    }

    /// Whether any component has pending visual changes. The main loop
    /// uses this after a finished job to tell real user-visible updates
    /// (e.g. an applied search filter) apart from housekeeping job
    /// completions that must not force a repaint.
    pub fn any_component_dirty(&self) -> bool {
        self.components
            .values()
            .chain(self.overlay.values())
            .any(|c| c.is_dirty())
    }

    /// Redraw bypassing the draw-rate limit. For one-shot events that
    /// apply state outside the timer chain (e.g. a finished search job
    /// applying its filter): the plain [`Self::redraw`] may fall inside
    /// the limiter's cooldown right after the keypress that started
    /// the job, and nothing repaints until the next keypress.
    pub fn redraw_force(&mut self) {
        self.draw_rate_limit.expire();
        self.redraw();
    }

    /// Force a redraw for all dirty components.
    pub fn redraw(&mut self) {
        if !self.draw_rate_limit.tick() {
            return;
        }

        log::debug!("redraw: begin");
        #[cfg(debug_assertions)]
        let __redraw_span = DrawSpan::enter("redraw total");

        for i in 0..self.components.len() {
            self.draw_component(i);
        }
        let mut areas: smallvec::SmallVec<[Area; 8]> =
            self.context.dirty_areas.drain(0..).collect();

        let can_draw_above_screen: bool = !matches!(self.mode, UIMode::Embedded | UIMode::Fork);
        if self.message_box.active {
            let now = datetime::now();
            if self
                .message_box
                .expiration_start
                .map(|t| t + 5 < now)
                .unwrap_or(false)
            {
                self.message_box.deactivate();
                areas.push(self.screen.area());
            }
        }

        /* Sort by x_start, ie upper_left corner's x coordinate */
        areas.sort_by(|a, b| a.upper_left().0.partial_cmp(&b.upper_left().0).unwrap());

        if self.message_box.active && can_draw_above_screen {
            /* Check if any dirty area intersects with the area occupied by
             * floating notification box */
            let displ = self.message_box.cached_area();
            let (displ_top, displ_bot) = (displ.upper_left(), displ.bottom_right());
            let mut is_dirty = self.message_box.is_dirty();
            for a in &areas {
                let (top_x, top_y) = a.upper_left();
                let (bottom_x, bottom_y) = a.bottom_right();
                is_dirty |= !(bottom_y < displ_top.1
                    || displ_bot.1 < top_y
                    || bottom_x < displ_top.0
                    || displ_bot.0 < top_x);
            }
            self.message_box.set_dirty(is_dirty);
        }
        // Overlay compositing: when a dialog overlay is present, the
        // dirty-area flush below must write from the overlay grid (which
        // contains the underlying content WITH the dialog composited on
        // top), not from the main grid. Writing from the main grid first
        // and compositing the overlay later produced a one-frame flash
        // of the underlying content on every timer tick (spinner,
        // listing refresh) - the flicker the user saw.
        let overlay_present = !self.overlay.is_empty() && can_draw_above_screen;
        if overlay_present {
            let area: Area = self.screen.area();
            if let Some((_, overlay_widget)) = self.overlay.get_index_mut(0) {
                let overlay_is_dirty = overlay_widget.is_dirty();
                let underlying_changed = !areas.is_empty();
                if overlay_is_dirty || underlying_changed {
                    {
                        let (grid, overlay_grid) = self.screen.grid_and_overlay_grid_mut();
                        overlay_grid.copy_area(grid, area, area);
                        overlay_widget.draw(overlay_grid, area, &mut self.context);
                    }
                    // Flush dirty rows from the composited overlay grid
                    // instead of the main grid: the overlay content is
                    // already in place, so there is no intermediate frame
                    // without the dialog.
                    let dirty_rows: Vec<usize> = if overlay_is_dirty {
                        (0..area.height()).collect()
                    } else {
                        let mut r: Vec<usize> = areas
                            .iter()
                            .flat_map(|a| a.upper_left().1..=a.bottom_right().1)
                            .collect();
                        r.sort_unstable();
                        r.dedup();
                        r
                    };
                    for y in dirty_rows {
                        if y < area.height() {
                            self.screen.draw_overlay(0..area.width(), y);
                        }
                    }
                    // Dirty areas were already flushed via the overlay
                    // grid; suppress the main-grid flush below.
                    areas.clear();
                }
            }
        }

        /* draw each dirty area */
        let rows = self.screen.area().height();
        for y in 0..rows {
            let mut segment = None;
            for ((x_start, y_start), (x_end, y_end)) in
                areas.iter().map(|a| (a.upper_left(), a.bottom_right()))
            {
                if y < y_start || y > y_end {
                    continue;
                }
                if let Some((x_start, x_end)) = segment.take() {
                    self.screen.draw(x_start..(x_end + 1), y);
                }
                match segment {
                    None => {
                        segment = Some((x_start, x_end));
                    }
                    Some((p_x_start, p_x_end)) if p_x_end < x_start => {
                        self.screen.draw(p_x_start..(p_x_end + 1), y);
                        segment = Some((x_start, x_end));
                    }
                    Some((p_x_start, p_x_end)) if p_x_end < x_end => {
                        self.screen.draw(p_x_start..(p_x_end + 1), y);
                        segment = Some((p_x_end, x_end));
                    }
                    Some((_, ref mut x)) => {
                        *x = x_end;
                    }
                }
            }
            if let Some((x_start, x_end)) = segment {
                self.screen.draw(x_start..(x_end + 1), y);
            }
        }

        if self.message_box.is_dirty() && self.message_box.active && can_draw_above_screen {
            if !self.message_box.is_empty() {
                if !self.message_box.initialised {
                    {
                        let cached_area = self.message_box.cached_area();
                        // Clear area previously occupied by floating notification box
                        if cached_area.generation() == self.screen.area().generation() {
                            for row in self.screen.grid().bounds_iter(cached_area) {
                                self.screen.draw(row.cols(), row.row_index());
                            }
                            let (grid, overlay_grid) = self.screen.grid_and_overlay_grid_mut();
                            overlay_grid.copy_area(grid, cached_area, cached_area);
                        }
                    }
                }
                let area = self.screen.area();
                self.message_box
                    .draw(self.screen.overlay_grid_mut(), area, &mut self.context);
                let cached_area = self.message_box.cached_area();
                // A cached area from before a resize belongs to a different
                // grid generation; iterating it would trip the `bounds_iter`
                // generation assert. Skip the frame (same guard as the
                // `message_box.active` clear-out path above).
                if cached_area.generation() == self.screen.overlay_grid().area().generation() {
                    for row in self.screen.overlay_grid().bounds_iter(cached_area) {
                        self.screen.draw_overlay(row.cols(), row.row_index());
                    }
                }
            }
            self.message_box.set_dirty(false);
        } else if self.message_box.is_dirty() && can_draw_above_screen {
            /* Clear area previously occupied by floating notification box */
            if self.message_box.cached_area().generation() == self.screen.area().generation() {
                for row in self
                    .screen
                    .grid()
                    .bounds_iter(self.message_box.cached_area())
                {
                    self.screen.draw(row.cols(), row.row_index());
                }
            }
            self.message_box.set_dirty(false);
        }

        self.flush();
    }

    /// Draw the entire screen from scratch.
    pub fn render(&mut self) {
        self.screen.update_size();
        self.context.dirty_areas.push_back(self.screen.area());

        self.redraw();
    }

    pub fn draw_component(&mut self, idx: usize) {
        let component = &mut self.components[idx];

        if component.is_dirty() {
            #[cfg(debug_assertions)]
            let __span = DrawSpan::enter(&format!("component[{idx}] {}", component));
            let area = self.screen.area();
            component.draw(self.screen.grid_mut(), area, &mut self.context);
            #[cfg(debug_assertions)]
            drop(__span);
        }
    }

    pub fn can_quit_cleanly(&mut self) -> bool {
        let Self {
            ref mut components,
            ref context,
            ..
        } = self;
        components.values_mut().all(|c| c.can_quit_cleanly(context))
    }

    pub fn register_component(&mut self, component: Box<dyn Component>) {
        component.realize(None, &mut self.context);
        self.components.insert(component.id(), component);
    }

    /// Convert user commands to actions/method calls.
    fn exec_command(&mut self, cmd: Action) {
        match cmd {
            SetEnv(key, val) => {
                // SAFETY: we only modify our environment from the main process/thread.
                std::env::set_var(key.as_str(), val.as_str());
            }
            PrintEnv(key) => {
                self.show_display_message(
                    std::env::var(key.as_str()).unwrap_or_else(|e| e.to_string()),
                );
            }
            ChangeCurrentDirectory(dir) => {
                self.context.current_dir = dir;
                self.show_display_message(self.context.current_dir.display().to_string());
            }
            CurrentDirectory => {
                self.show_display_message(self.context.current_dir.display().to_string());
            }
            Mailbox(account_name, op) => {
                if let Some(account) = self
                    .context
                    .accounts
                    .values_mut()
                    .find(|a| a.name() == account_name)
                {
                    if let Err(err) = account.mailbox_operation(op) {
                        self.context.replies.push_back(UIEvent::Notification {
                            title: None,
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                } else {
                    self.context.replies.push_back(UIEvent::Notification {
                        title: None,
                        source: None,
                        body: format!("Account with name `{account_name}` not found.").into(),
                        kind: Some(NotificationType::Info),
                    });
                }
            }
            #[cfg(feature = "sqlite3")]
            AccountAction(ref account_name, ReIndex) => {
                let account_index = if let Some(a) = self
                    .context
                    .accounts
                    .iter()
                    .position(|(_, acc)| acc.name() == account_name)
                {
                    a
                } else {
                    self.context.replies.push_back(UIEvent::Notification {
                        title: None,
                        source: None,
                        body: format!("Account {account_name} was not found.").into(),
                        kind: Some(NotificationType::Error(ErrorKind::None)),
                    });
                    return;
                };
                if *self.context.accounts[account_index]
                    .settings
                    .conf
                    .search_backend()
                    != SearchBackend::Sqlite3
                {
                    self.context.replies.push_back(UIEvent::Notification {
                        title: None,
                        source: None,
                        body: format!(
                            "Account {account_name} doesn't have an sqlite3 search backend.",
                        )
                        .into(),
                        kind: Some(NotificationType::Error(ErrorKind::None)),
                    });
                    return;
                }
                let account = &self.context.accounts[account_index];
                let (acc_name, backend_mutex): (Arc<str>, Arc<_>) =
                    (Arc::clone(&account.name), account.backend.clone());
                let job = crate::sqlite3::AccountCache::index(
                    acc_name,
                    account.collection.clone(),
                    backend_mutex,
                );
                let handle = self.context.main_loop_handler.job_executor.spawn(
                    "sqlite3::index".into(),
                    job,
                    crate::sqlite3::AccountCache::is_async(),
                );
                self.context.accounts[account_index].active_jobs.insert(
                    handle.job_id,
                    crate::accounts::JobRequest::Generic {
                        name: "Message index rebuild".into(),
                        handle,
                        on_finish: None,
                        log_level: LogLevel::INFO,
                    },
                );
                self.context.replies.push_back(UIEvent::Notification {
                    title: None,
                    source: None,
                    body: "Message index rebuild started.".into(),
                    kind: Some(NotificationType::Info),
                });
            }
            #[cfg(not(feature = "sqlite3"))]
            AccountAction(_, ReIndex) => {
                self.context.replies.push_back(UIEvent::Notification {
                    title: None,
                    source: None,
                    body: "Message index rebuild failed: meli is not built with sqlite3 support."
                        .into(),
                    kind: Some(NotificationType::Error(ErrorKind::None)),
                });
            }
            AccountAction(ref account_name, PrintAccountSetting(ref setting)) => {
                let path = setting.split('.').collect::<SmallVec<[&str; 16]>>();
                if let Some(pos) = self
                    .context
                    .accounts
                    .iter()
                    .position(|(_h, a)| a.name() == account_name)
                {
                    self.context.replies.push_back(UIEvent::StatusEvent(
                        StatusEvent::UpdateStatus(
                            self.context.accounts[pos]
                                .settings
                                .lookup("settings", &path)
                                .unwrap_or_else(|err| err.to_string()),
                        ),
                    ));
                } else {
                    self.context.replies.push_back(UIEvent::Notification {
                        title: None,
                        source: None,
                        body: format!("Account {account_name} was not found.").into(),
                        kind: Some(NotificationType::Error(ErrorKind::None)),
                    });
                }
            }
            PrintSetting(ref setting) => {
                let path = setting.split('.').collect::<SmallVec<[&str; 16]>>();
                self.context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::UpdateStatus(
                        self.context
                            .settings
                            .lookup("settings", &path)
                            .unwrap_or_else(|err| err.to_string()),
                    )));
            }
            ToggleMouse => {
                let new_val = !self.screen.tty().mouse();
                self.screen.tty_mut().set_mouse(new_val);
                self.rcv_event(UIEvent::StatusEvent(StatusEvent::SetMouse(new_val)));
            }
            ToggleTheme => {
                self.open_theme_picker();
            }
            Quit => {
                self.context.replies.push_back(UIEvent::Exit);
            }
            #[cfg(feature = "cli-docs")]
            Tab(Man(manpage)) => match manpage
                .read(false)
                .map(|text| crate::manpages::ManPages::remove_markup(&text).unwrap_or(text))
            {
                Ok(m) => self.rcv_event(UIEvent::Action(Tab(New(Some(Box::new(
                    Pager::from_string(
                        m,
                        &self.context,
                        None,
                        None,
                        crate::conf::value(&self.context, "theme_default"),
                    ),
                )))))),
                Err(err) => self.context.replies.push_back(UIEvent::Notification {
                    title: None,
                    body: "Encountered an error while retrieving manual page.".into(),
                    source: Some(err),
                    kind: Some(NotificationType::Error(ErrorKind::Bug)),
                }),
            },
            v => {
                self.rcv_event(UIEvent::Action(v));
            }
        }
    }

    /// The application's main loop sends `UIEvents` to state via this method.
    pub fn rcv_event(&mut self, mut event: UIEvent) {
        #[cfg(debug_assertions)]
        let __rcv_span = DrawSpan::enter(&format!(
            "rcv_event {}",
            format!("{event:?}").chars().take(120).collect::<String>()
        ));
        // The theme picker's live-preview / persistence requests are
        // State-level concerns: apply (and optionally persist) the theme,
        // then let the ConfigReload-style refresh below reach every
        // component.
        if let UIEvent::ChangeTheme { name, persist } = event {
            self.apply_theme(name, persist);
            return;
        }
        if let UIEvent::Input(_) = event {
            if self.message_box.expiration_start.is_none() {
                self.message_box.expiration_start = Some(datetime::now());
            }
        }

        // Exit requests are recorded for the main loop; components must
        // not see (or re-handle) them.
        if matches!(event, UIEvent::Exit) {
            self.exit_requested = true;
            return;
        }

        match event {
            // Command type is handled only by State.
            UIEvent::Command(cmd) => {
                match parse_command(cmd.as_bytes()) {
                    Ok(action) => {
                        if action.needs_confirmation() {
                            let new = Box::new(UIConfirmationDialog::new(
                                "Are you sure?",
                                vec![(true, "yes".to_string()), (false, "no".to_string())],
                                true,
                                Some(Box::new(move |id: ComponentId, result: bool| {
                                    Some(UIEvent::FinishedUIDialog(
                                        id,
                                        Box::new(if result { Some(action) } else { None }),
                                    ))
                                })),
                                &self.context,
                            ));

                            self.overlay.insert(new.id(), new);
                        } else if matches!(action, Action::ReloadConfiguration) {
                            let res = Settings::new().and_then(|new_settings| {
                                let old_accounts = self
                                    .context
                                    .settings
                                    .accounts
                                    .keys()
                                    .collect::<std::collections::HashSet<&String>>();
                                let new_accounts = new_settings
                                    .accounts
                                    .keys()
                                    .collect::<std::collections::HashSet<&String>>();
                                if old_accounts != new_accounts {
                                    return Err("cannot reload account configuration changes; \
                                                restart meli instead."
                                        .into());
                                }
                                for (key, acc) in new_settings.accounts.iter() {
                                    if toml::Value::try_from(acc)
                                        != toml::Value::try_from(
                                            &self.context.settings.accounts[key],
                                        )
                                    {
                                        return Err("cannot reload account configuration \
                                                    changes; restart meli instead."
                                            .into());
                                    }
                                }
                                if toml::Value::try_from(&new_settings)
                                    == toml::Value::try_from(&self.context.settings)
                                {
                                    return Err("No changes detected.".into());
                                }
                                Ok(Box::new(new_settings))
                            });
                            match res {
                                Ok(new_settings) => {
                                    let old_settings =
                                        std::mem::replace(&mut self.context.settings, new_settings);
                                    self.context
                                        .replies
                                        .push_back(UIEvent::ConfigReload { old_settings });
                                    self.context.replies.push_back(UIEvent::Resize);
                                }
                                Err(err) => {
                                    self.context.replies.push_back(UIEvent::Notification {
                                        title: Some("Could not load configuration".into()),
                                        source: None,
                                        body: err.to_string().into(),
                                        kind: Some(NotificationType::Error(err.kind)),
                                    });
                                }
                            }
                        } else {
                            self.exec_command(action);
                        }
                    }
                    Err(err) => {
                        self.context.replies.push_back(UIEvent::Notification {
                            title: Some(format!("Invalid command `{cmd}`").into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(ErrorKind::ValueError)),
                        });
                    }
                }
                return;
            }
            UIEvent::Fork(
                child @ ForkedProcess::Generic {
                    id: _,
                    command: _,
                    child: _,
                },
            ) => {
                let id = child.id().to_string();
                self.context
                    .children
                    .entry(id.into())
                    .or_default()
                    .push(child);
                return;
            }
            UIEvent::Fork(child) => {
                self.mode = UIMode::Fork;
                self.child = Some(child);
                return;
            }
            UIEvent::BackendEvent(
                account_hash,
                BackendEvent::Notice {
                    ref description,
                    ref content,
                    level,
                },
            ) => {
                let account_name = self
                    .context
                    .accounts
                    .get(&account_hash)
                    .map(|a| a.name())
                    .unwrap_or("unknown account");
                let msg = format!(
                    "{}: {}{}{}",
                    account_name,
                    description.as_str(),
                    if content.is_some() { ": " } else { "" },
                    content.as_ref().map(|s| s.as_str()).unwrap_or("")
                );
                log::log!(level.into(), "{msg}");
                self.show_display_message(msg);
                return;
            }
            UIEvent::BackendEvent(account_hash, BackendEvent::AccountStateChange { message }) => {
                self.rcv_event(UIEvent::AccountStatusChange(account_hash, Some(message)));
                return;
            }
            UIEvent::BackendEvent(
                _,
                BackendEvent::Refresh(RefreshEvent {
                    account_hash,
                    mailbox_hash,
                    kind,
                }),
            ) => {
                self.refresh_event(account_hash, mailbox_hash, vec![kind]);
                return;
            }
            UIEvent::BackendEvent(_, BackendEvent::RefreshBatch(events)) => {
                let Some(first) = events.first() else {
                    return;
                };
                let account_hash = first.account_hash;
                let mailbox_hash = first.mailbox_hash;
                let Some(events) = events
                    .into_iter()
                    .map(|ev| {
                        if (ev.account_hash, ev.mailbox_hash) != (account_hash, mailbox_hash) {
                            None
                        } else {
                            Some(ev.kind)
                        }
                    })
                    .collect::<Option<Vec<RefreshEventKind>>>()
                else {
                    return;
                };
                self.refresh_event(account_hash, mailbox_hash, events);
                return;
            }
            UIEvent::ChangeMode(m) => {
                self.context
                    .main_loop_handler
                    .sender
                    .send(ThreadEvent::UIEvent(UIEvent::ChangeMode(m)))
                    .unwrap();
            }
            UIEvent::Timer(id) if id == self.draw_rate_limit.id() => {
                self.draw_rate_limit.reset();
                self.redraw();
                return;
            }
            UIEvent::Input(ref key)
                if self
                    .context
                    .settings
                    .shortcuts
                    .general
                    .info_message_previous
                    .contains(key) =>
            {
                self.message_box.show_previous();
                return;
            }
            UIEvent::Input(ref key)
                if self
                    .context
                    .settings
                    .shortcuts
                    .general
                    .info_message_next
                    .contains(key) =>
            {
                self.message_box.show_next();
                return;
            }
            UIEvent::Notification {
                ref title,
                source: _,
                ref body,
                kind: _,
            } if self.context.settings.notifications.enable.ui_enabled() => {
                self.show_display_message(format!(
                    "{title}{}{body}",
                    if title.is_some() { " " } else { "" },
                    title = title.as_deref().unwrap_or_default(),
                    body = body,
                ));
            }
            UIEvent::FinishedUIDialog(ref id, ref mut results) if self.overlay.contains_key(id) => {
                if let Some(ref mut action @ Some(_)) = results.downcast_mut::<Option<Action>>() {
                    self.exec_command(action.take().unwrap());

                    return;
                }
            }
            UIEvent::Callback(callback_fn) => {
                (callback_fn.0)(&mut self.context);
                return;
            }
            UIEvent::GlobalUIDialog { value, parent } => {
                self.context.realized.insert(value.id(), parent);
                self.overlay.insert(value.id(), value);
                self.process_realizations();
                return;
            }
            UIEvent::ProcessRequest {
                owner,
                mut command,
                spawn,
                result_cb,
            } => {
                log::trace!(
                    "Executing: {:?} {:?}",
                    command.get_program(),
                    command.get_args().collect::<Vec<_>>()
                );
                let content = if let Some(spawn_fn) = spawn {
                    // Kill input thread so that spawned command can be sole receiver of stdin
                    self.context.input_kill();
                    // Restore the original blocking stdin in case the
                    // input thread has not exited its fd 0 nonblocking
                    // swap yet: the child must not inherit the swapped
                    // descriptor.
                    crate::terminal::input::restore_stdin_for_child_spawn();

                    self.screen.switch_to_main_screen();
                    let result = command.spawn().map_err(Into::into).and_then(|child| {
                        let child = (spawn_fn.0)(child)?;

                        child
                            .wait_with_output()
                            .map_err(Into::into)
                            .and_then(|output| {
                                let status = output.status;
                                if status.success() {
                                    return Ok(output);
                                }
                                Err(Error::new(match status.code() {
                                    Some(code) => {
                                        format!("Process exited with status code: {code}")
                                    }
                                    None => "Process terminated by signal".to_string(),
                                })
                                .set_details(format!("Captured output was: {output:?}")))
                            })
                    });
                    self.screen.switch_to_main_screen();
                    self.screen.switch_to_alternate_screen(&self.context);
                    self.context.restore_input();
                    (result_cb.0)(result)
                } else {
                    (result_cb.0)(command.output().map_err(Into::into))
                };
                if let Some(content) = content {
                    if content.is::<UIEvent>() {
                        self.rcv_event(*content.downcast::<UIEvent>().unwrap());
                    } else {
                        self.rcv_event(UIEvent::IntraComm {
                            from: owner,
                            to: owner,
                            content,
                        });
                    }
                }
                return;
            }
            _ => {}
        }

        self.process_realizations();

        let Self {
            ref mut components,
            ref mut context,
            ref mut overlay,
            ..
        } = self;

        /* inform each component */
        for c in overlay.values_mut().chain(components.values_mut()) {
            if c.process_event(&mut event, context) {
                break;
            }
        }

        if !self.context.replies.is_empty() {
            let replies: smallvec::SmallVec<[UIEvent; 8]> =
                self.context.replies.drain(0..).collect();
            // Pass replies to self and call count on the map iterator to force evaluation
            replies.into_iter().map(|r| self.rcv_event(r)).count();
        }
    }

    #[inline]
    fn show_display_message(&mut self, msg: String) {
        if self.message_box.try_push(msg, datetime::now()) {
            self.message_box.arm(&mut self.context);
            if self.message_box.is_dirty() {
                self.redraw();
            }
        }
    }

    fn process_realizations(&mut self) {
        while let Some((id, parent)) = self.context.realized.pop() {
            match parent {
                None => {
                    self.component_tree.insert(id, ComponentPath::new(id));
                }
                Some(parent) if self.component_tree.contains_key(&parent) => {
                    let mut v = self.component_tree[&parent].clone();
                    v.push_front(id);
                    if let Some(p) = v.root() {
                        assert_eq!(
                            v.resolve(&self.components[p] as &dyn Component)
                                .unwrap()
                                .id(),
                            id
                        );
                    }
                    self.component_tree.insert(id, v);
                }
                Some(parent) if !self.context.realized.contains_key(&parent) => {
                    log::debug!(
                        "BUG: component_realize new_id = {:?} parent = {:?} but component_tree \
                         does not include parent, skipping.",
                        id,
                        parent
                    );
                    self.component_tree.insert(id, ComponentPath::new(id));
                }
                Some(_) => {
                    let from_index = self.context.realized.len();
                    self.context.realized.insert(id, parent);
                    self.context.realized.move_index(from_index, 0);
                }
            }
        }

        while let Some(id) = self.context.unrealized.pop() {
            let mut to_delete = BTreeSet::new();
            for (desc, _) in self.component_tree.iter().filter(|(_, path)| {
                path.parent()
                    .map(|p| self.context.unrealized.contains(p) || *p == id)
                    .unwrap_or(false)
            }) {
                to_delete.insert(*desc);
            }
            self.context.unrealized.extend(to_delete);
            self.component_tree.shift_remove(&id);
            self.components.shift_remove(&id);
            self.overlay.shift_remove(&id);
        }
    }

    /// Try wait on `self.child` without blocking.
    ///
    /// Return value is:
    ///
    /// - `None` if there's no child to wait for.
    /// - `Some(true)` if the child has exited.
    /// - `Some(false)` if the child is still running.
    pub fn try_wait_on_main_child(&mut self) -> Option<bool> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(false) => Some(false),
            ws @ Ok(true) | ws @ Err(_) => {
                if let Err(err) = ws {
                    log::error!("{}", err);
                }
                if matches!(child, ForkedProcess::Embedded { .. }) {
                    // The check on whether the embedded process is alive is done on input, so
                    // forward an input of '\0' to get the embedded terminal to notice its
                    // child is dead.
                    // [ref:TODO]: replace with a "PidExited" event or similar.
                    self.rcv_event(UIEvent::EmbeddedInput((Key::Null, vec![0])));
                }
                self.child = None;
                Some(true)
            }
        }
    }

    /// Try waiting on `self.child` or `self.context`'s children.
    pub fn try_wait_on_children(&mut self) {
        if !matches!(self.try_wait_on_main_child(), Some(true)) {
            for (id, children) in self.context.children.iter_mut() {
                let mut i = 0;
                while i < children.len() {
                    match children[i].try_wait() {
                        Ok(false) => {
                            i += 1;
                        }
                        ws @ Ok(true) | ws @ Err(_) => {
                            if let Err(err) = ws {
                                log::error!(
                                    "Child {}:{:?} could not be waited for: {}",
                                    id,
                                    children[i],
                                    err
                                );
                            }
                            log::trace!("Child {}:{:?} has exited.", id, children[i]);
                            children.remove(i);
                        }
                    }
                }
            }
            let mut i = 0;
            while i < self.context.children.len() {
                if self.context.children[i].is_empty() {
                    self.context.children.swap_remove_index(i);
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Force kill `self.child`, if it exists.
    ///
    /// Return value is:
    ///
    /// - `None` if there's no child to kill.
    /// - `Some(Ok(()))` if the child is no longer running.
    /// - `Some(Err(_))` if an error occurred.
    pub fn kill_main_child(&mut self) -> Option<Result<()>> {
        Some(self.child.as_mut()?.kill())
    }

    /// Switch back to the terminal's main screen (The command line the user
    /// sees before opening the application)
    pub fn switch_to_main_screen(&mut self) {
        self.screen.switch_to_main_screen();
    }

    pub fn switch_to_alternate_screen(&mut self) {
        self.screen.switch_to_alternate_screen(&self.context);
    }

    fn flush(&mut self) {
        self.screen.flush();
    }

    pub fn check_accounts(&mut self) {
        let mut ctr = 0;
        for i in 0..self.context.accounts.len() {
            if self.context.is_online_idx(i).is_ok() {
                ctr += 1;
            }
        }
        if ctr != self.context.accounts.len() {
            self.timer.thread().unpark();
        }
        self.context.input_thread.check();
    }

    pub fn pulse(&mut self) {
        self.check_accounts();
        self.redraw();
    }
}
