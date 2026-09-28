<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

[English](./CHANGELOG.md) | **简体中文**

# 更新日志（Changelog）

本项目所有重要变更都会记录在本文件中。本文件为简体中文版，与英文版
[CHANGELOG.md](./CHANGELOG.md) 内容保持同步：新增条目时请同时更新两个文件。
fork 自身版本（`[Unreleased]`、`[v0.9.0]`）提供完整中文对照；fork 所基于的
上游历史版本（v0.8.13 及更早）为上游原始发布记录，未做翻译，请查阅英文版。

格式基于 [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)，
版本号遵循[语义化版本](https://semver.org/spec/v2.0.0.html)。

## [Unreleased]

### 新增（Added）

- 状态栏快捷键提示随 listing 布局与键盘焦点变化（计划 `statusbar-hints-per-layout`）：右段 hints 现按新增的 `Component::hint_focus()` trait 方法过滤（默认 `None`，`Tabbed` 转发活跃子组件的返回值，`Listing` 上报所在布局窗格）。layout1（无打开视图）删去 `Search` 提示；layout2/layout4 的邮件详情态（邮件视图持键盘）删去 `Scroll Up` / `Scroll Down` / `Focus Left` / `Focus Right` / `Search`，改为显示三个邮件动作——`Reply`（`envelope-view.reply`）、`Reply All`（`envelope-view.reply_to_all`）与 `Open in Tab`（`thread-view.open_in_new_tab`），`Quit` 恒为最后一项。各状态下按键行为不变，仅提示随之增减；提示与按键派发读同一份配置绑定，重绑快捷键后自动同步。

- 底栏三段式重构（计划 `statusbar-gauge-spinner`）：底栏单行经 `Layout::horizontal` 切分为左状态段、中部 `LineGauge` 段、右 hints 段。中部 `LineGauge` 由焦点邮箱的 `MailboxStatus::Parsing(done, total)` 驱动，按 `done/total` 推进并显示 `Fetch N/T` 标签；右段从 `Component::shortcuts()` 实时聚合 `q:quit ?:toggle_help F5:refresh`（行窄时按 `…` / `...` 截断）。左段右缘新增后端 chip `[maildir]` / `[imap ✓]` / `[imap ✘]` 与焦点邮箱标签 `📩 INBOX:42`，均由 `context.accounts` 加新的 `StatusBar.focus: Option<(AccountHash, MailboxHash)>` 字段在 `draw` 时现算，不引入新 theme key。ASCII 终端以 `+`/`x` 替代 ✓/✘ 并去掉信封 emoji；中部 gauge 使用 `filled_symbol # / .` 与 `status.bar` 反色（与 Insert 模式指示器共用同一调色板分叉）。

- 底栏数据通道。`StatusEvent` 新增 `FocusMailbox(AccountHash, MailboxHash)`，由当前 listing/树组件报告用户焦点；`Component` trait 新增 `status_watch()` 默认 `None`，`Listing` 覆写为返回光标 `(AccountHash, MailboxHash)`，`Tabbed` 转发活跃子组件的返回值。`StatusBar` 消费 `FocusMailbox` 记录焦点；同时新增**非消费**的 `MailboxUpdate((acc, mb))` 与 `AccountStatusChange(acc, _)` 事件臂，仅在事件 `AccountHash` 命中焦点账号时标脏，非焦点刷新不再触发底栏重绘。`Listing` 内 8 处、`Tabbed` 内 2 处 `UpdateStatus` 发送点统一收敛到一个 helper，同时发出字符串与结构化焦点事件。

- 输入线程私有 CSI 看门狗：检测「字节被 crossterm 消费但无事件产生」的卡死（如 mux/终端迟到的 `CSI ?` 私有序列回复在 crossterm 0.29 解析器内无限缓冲吞键），3s 停滞 + 2s 输入静默判定后自动注入 DA1 查询（`ESC[c`）触发终端回复使解析器整包清缓冲，恢复后续按键（被吞按键不可恢复）；连续 3 次注入无恢复则进程内停用；正常使用零注入。输入循环主等待重构为 `poll(2)`，输入线程将 stdin 换为非阻塞 tty 描述以避免 crossterm 内部读取死锁。新增 `scripts/test-private-csi-watchdog.sh` PTY 回归门（tmux 缺失时 SKIP）。

- Composer 新增关闭快捷键（`[shortcuts.composing]` 的 `close` 键，默认 `Esc`）：在邮件编辑界面按 `Esc` 关闭标签页返回之前的界面，草稿有未保存修改时弹出 x/y/n 对话框（不保存退出 / 保存草稿退出 / 取消）；附件管理模式按 `Esc` 返回主编辑界面。同步补齐移除附件、附件内层保存、收件人确认、gpg 密钥确认四处 `has_changes` 缺口，避免 Esc 关闭时静默丢弃未保存修改（计划 `composer-esc-exit`）。

- 命令面板（VSCode Ctrl+P 风格，计划 `command-palette`）：按 `:` / `M-x` 不再展开底部第二行命令条，而是在当前界面上弹出居中浮动面板（四角位于屏幕 10%/90%，即 80% × 80%）。面板顶部为单行输入框（`ratatui-textarea`，支持退格/方向键/Home/End），下方为 nucleo 模糊匹配列表（`nucleo-matcher`，智能大小写，按分数降序），候选为 `COMMAND_COMPLETION` 全部命令 + 历史记录，命中字符加粗+下划线高亮。空查询时显示 `Frequent`（使用超过一次的命令，按次数优先，取前 10）与 `History`（新→旧去重，取前 10）两节——无历史时列出全部命令。按键：`Tab` 将输入补全为选中项，`Enter` 执行选中项（或原始输入）并关闭，`Up`/`Down`（`Ctrl-P`/`Ctrl-N`）移动选择，`Esc` 直接关闭不执行。执行路径不变：面板只入队 `UIEvent::Command`，解析、`needs_confirmation` 确认框（如 quit）、历史记录（`cmd_history`）行为与之前完全一致；listing 的 `/` 与 F3 搜索预填仍然落入输入框。同时从 `StatusBar` 移除旧的 ex-buffer/`AutoComplete` 管线与随之死掉的 token 补全机器（`Token`/`TokenStream`/`command_completion_suggestions`）；命令模式不再占用底部第二行，仅保留单行状态行（模式指示仍显示 COMMAND）。新依赖：`nucleo-matcher` 0.3 与 `ratatui-textarea` 0.9（禁用默认 feature，不引入终端后端；`ratatui-core`/`ratatui-widgets` 版本与既有 ratatui 0.30 门面统一）。`TextArea` 因渲染缓存为 `!Sync` 内部可变单元而包在 `Mutex` 后。

- `open_in_new_tab` thread-view 快捷键（默认 `Enter`）：邮件内容面板持有键盘时按下——即所有 layout 的邮件视图（双栏邮件布局的右栏、thread 布局的邮件面板）——将当前正在阅读的邮件在新标签页打开，与 `open-in-tab` 命令完全同一动作（ThreadView 把按键重派发为 `ListingAction::OpenInNewTab`）。邮件视图弹窗（URL 启动、List-Unsubscribe 确认等）打开时 Enter 仍归弹窗；线程列表或邮件网格持键盘时该快捷键不生效。可在 `[shortcuts.thread-view]` 配置。

- 邮件操作全量命令化（计划 `command-palette-mail-ops`）：所有邮件操作均可从命令面板 / `:` 命令栏唤起。新增命令：`reply`、`reply all`、`reply author`、`forward`（按 `composing.forward_as_attachment` 弹 inline/附件选择）、`forward inline`、`forward attachment`、`new-mail`（空白编辑器）、`open`（打开列表光标处邮件/会话，同 `Enter`）、`refresh`（刷新焦点邮箱，同 `F5`：列表焦点刷当前打开邮箱、侧栏焦点刷高亮邮箱）；并把此前仅有解析器的 `flag` 命令补入补全表。回复/转发与键盘快捷键走完全相同的代码路径：已打开会话视图时动作直达该视图，否则先打开光标处条目再转发；在列表上作用于选中邮件，在邮件视图上作用于正在阅读的邮件。

- PGP 可插拔后端（上游 `89f834b6` + `97f02477` + `7110e8d0`，语义移植）：`melib::email::pgp` 新增 `PGPBackend` trait（sign/verify/encrypt/decrypt/keylist/get_key）与 `Key` 抽象；gpgme 降为该 trait 的一个实现，另新增 `cli` 后端——按 `[pgp] backend = "cli"` 配置执行外部辅助脚本（含 `display_name`/`scan_command` 等键），`contrib/pgp-cli-backends/gpg/` 附带六个 GnuPG 参考脚本。`compose/gpg.rs` 更名 `compose/pgp.rs` 并改走 trait；keylist 结果以 `IndexSet` 去重；后端反序列化的错误提示修正。fork 偏差：cleartext 验证管线（`UnverifiedSignature`/`extract_unverified_signature`/`Context::verify_cleartext`、SignedPending→SignedVerified 路由）逐字保留，不引入上游 ViewFilter/FilterOutputMetadata 机制；无 `gpgme` feature 时默认 CLI 后端且 compose 签名/加密开关保持隐藏（延续 fork 的 no-gpgme UI 契约）；`conf/overrides.rs` 经 sentinel 重新生成。

- JMAP EventSource 推送（上游 `40a45b04` + `af619461`，语义移植）：RFC 8620 URI 模板展开抽为独立模块 `melib/src/jmap/url_template.rs`（含单测）；新增 `melib/src/jmap/eventsource.rs` 对 session 的 `eventSourceUrl` 维持 SSE 长连接，按推送的 `State` 变化触发重同步，取代轮询。fork 偏差：协议违规返回 `ErrorKind::ProtocolError` 而非 panic；SSE 请求用 `RedirectPolicy::None` 延续 fork 的跨源重定向凭据加固；fork 的 JMAP 测试插桩（`error_responses`、since_state==current → 空响应）保留，并新增 `test_jmap_watch` SSE mock 服务器用例。

### 变更（Changes）

- 依赖治理：移除根 `Cargo.toml` 的 `[patch.crates-io]` 与 `vendor/crossterm/` 目录，crossterm 回归 crates.io 0.29.0 原版。原补丁防御的「未识别私有 CSI 卡死」改由删除无用启动查询（`CSI ? 2026 $ p` 同步输出支持探测，全代码库无消费者）+ 输入看门狗承担。离线构建改为依赖本地 cargo cache 预取（`cargo fetch`）。
- `envelope-view.reply_to_all` 默认键由 `C-g` 改为 `C-a`（`reply` 保持 `r`，`reply_to_author` 保持 `C-r`）；底栏 `Reply All` 提示与邮件视图按键派发随同一配置绑定自动同步。

### 修复（Fixed）

- IMAP 离线缓存遍历改为按行分页，消除 163/Coremail 的刷新风暴（状态栏图标常转）：`CacheFirst`/`FromCache` 抓取阶段原先按 UID 空间做 `max_uid -= batch_size` 步进，在长寿服务器的稀疏 UID 空间（`UIDVALIDITY = 1`、`uidnext` 达 10^8-10^9、真实邮件只有少量）下一次抓取要迭代数千个几乎全空的缓存窗口——每个窗口都是一次瞬时 sqlite 查询并发出一个 `MailboxUpdate` payload——状态栏邮箱图标因此永久转个不停（日志证据：四个邮箱整场会话合计约每分钟 5 万个 `MailboxUpdate`，每次抓取约 600 个瞬时 `fetch-mailbox-continued` chunk）。`ImapCache::envelopes` 现按 `ORDER BY uid DESC LIMIT ?` 返回最新 `batch_size` 行并附带本页最低 UID（隔离区占位行随同页窗口一并服务），两个阶段的下一页都推进到 `最低 UID − 1`，页面为空或抵达 UID 1 即结束：无论 UID 空间多稀疏，遍历只需 `O(缓存行数 / batch_size)` 次查询，首次抓取数秒内完成，之后只做增量同步（验收目标：刷新完成后安静，仅剩定时 watch/IDLE）。

- 后台账号刷新不再把布局拉回 layout1（163/IMAP 重连场景）：当前账号的 `UIEvent::AccountStatusChange`（看门狗重连时的「Establishing TLS connection.」「Attempting authentication.」，或重连后的邮箱列表对账「Refreshed mailboxes.」）此前会完整执行 `Listing::change_account`，其无条件的 `close_view` 会把屏幕上的 layout2/3/4 塌回 layout1。现在对账路径保留已打开的视图：`change_account` 新增 `keep_view` 标志（跳过 `close_view` 拆除；邮箱未变时跳过 `set_coordinates`——它会重置网格的条目焦点与过滤状态，使打开的视图悬在无焦点网格上，而视图的绘制门是 `component.unfocused()`）；`set_index_style` 在样式无变化时不再关闭视图。若对账把网格落到别的邮箱或光标下已无条目，过期视图仍会关闭并落回 layout1；layout4 的 `View` 键盘焦点随视图存活。刷新重踢 `OpenEntryUnderCursor` 时，`set_grid_focused` / `set_grid_has_keyboard` 现在尊重存活的 `View` 焦点。回归测试：`account_status_change_keeps_open_view`（layout2，含邮件面板实际渲染的 draw 断言）、`account_status_change_keeps_view_focus`（layout4）、`account_status_change_closes_stale_view_on_moved_mailbox`（过期关闭）。
- 退回 layout1 时键盘落在邮箱列表（mailbox list）：从任意会关闭视图退回 layout1 的布局（layout2 的网格或邮件视图、layout3 的网格）按退出键（`exit_entry` 的 `i` 与通用 quit 的 `q`/`Esc`）原先把键盘留在邮件网格上；现在只要侧栏可见，键盘固定落在邮箱侧栏——与 `focus_left` 的落点一致。侧栏隐藏（`menu_visibility = false`）时仍落网格；中间步骤（layout4 → layout3）不变。`conversations_entry_close_no_residue` golden 已重录（侧栏 ring 亮、网格 ring 暗）。测试：`quit_key_exits_open_mail_view`（新增断言 `Menu` 落点）、`layout3_left_goes_to_mailbox_list_and_l4_quit`（layout3 退出落邮箱列表且侧栏可见）。

- 邮件正文标签页（`MailViewTab`）带 ratatui 圆角边框：`open_in_new_tab` 引入的全屏邮件标签页现在在正文四周绘制圆角边框（`draw_rounded_frame`，即 ratatui `Block::bordered` 圆角 border set 的桥接），主题 "tab.focused"、内容底色 "pane.focused"——与其他标签页内容的 pane-ring 约定一致。边框由一个薄包装组件实现，其余（事件、脏标记、kill、快捷键、realize 组件树）全部委托给内层 `MailView`。回归测试：`enter_at_mail_view_focus_opens_new_tab` 断言标签页 payload 渲染出圆角（`╭`/`╯`）且内部无会话列表行。
- `open_in_new_tab` 打开全屏邮件内容（layout4 修复）：`ThreadView` 的 `ListingAction::OpenInNewTab` 分支改为按焦点分派。邮件面板持键盘时（`open_in_new_tab` 快捷键，layout 2/4）新标签页是正在阅读邮件的 `MailView`——全屏邮件内容，即 `MailView` 自身命令分支一直产出的组件（"opens envelope view in new tab"）；此前转发源焦点、直接以 `MailView` 焦点构造 `ThreadView` 的做法在标签页里首帧渲染错乱。线程列表状态（`open-in-tab` 命令路径）仍打开整线程视图的整列表状态标签页，行为不变。测试：`enter_at_mail_view_focus_opens_new_tab` 同时绘制两种 payload——快捷键标签页无会话列表 chrome，命令标签页保持整列表框与行。

- IMAP `resync_condstore` 空序列集边界（上游 `ed162e11`）：`lastseenuid == 1` 时 `(1..lastseenuid)` 生成空区间 `1..1`，被 imap_types 以 `ValidationError` 拒绝；`tag2 UID FETCH … FLAGS` 阶段改为 `lastseenuid < 2` 时使用全区间 `1:*`。`resync_basic` 处为闭区间 `..=`，经核对不受影响。

- 标签重命名哈希（上游 `d8cc16b9`）：`TagName` 的 `Hash` impl 原先哈希显示名而非标签哈希，两个不同标签同名时 `[tags] rename` 映射查表错乱；现改为哈希 `TagHash`（字段改 `pub` 供测试构造），附 `test_conf_tag_rename` 回归测试。

- 私用区（PUA）字符宽度（上游 `3f8427b0`）：PUA 码位（Nerd Fonts 等图标字体大量使用）原先被判为不可打印（`None`）；现归入 Ambiguous 宽度（1 格），附回归测试。

- compact 列表标签边距（上游 `6d3dd4bd`）：标签文本现在从 `area_col_4.skip_cols(1)` 起打印，补齐此前"留 1 格"修复的最后一处。

- 通知框边框残留（上游 `c498d7e6`）：清屏循环现遍历完整 `cached_area`（含边框环，适配 fork 的 `draw_rounded_frame`）后再画框，高亮属性不再残留于边框。

- 编辑器启动（上游 `59c5ad4b` + `bb6d5916`）：编辑器解析优先级为 `composing.editor_command` > `$VISUAL` > `$EDITOR`（未设置提示文案同步更新）；草稿路径改为经 argv 传参（`sh -c '<editor> "$@"' -- editor path`），不再字符串拼接——路径含空格或 shell 元字符不再破坏启动，草稿文件名也无法注入 shell 语法。fork 的临时文件名长度加固保持不变。

### 已知问题（Known Issues）

- 看门狗已知限制（与 vendor 补丁时代的非回归项一致或为新增残留）：`<`（SGR 鼠标半截序列）卡死不受 DA1 注入保护；迟到 OSC 10/11 回复产生的垃圾键为正常事件、看门狗对其失明；mux 以 <2s 间隔持续注入 `CSI ?` 回复时看门狗不触发；极慢链路大粘贴理论上有 DA1 字节混入风险；卡死恢复延迟最坏 ≈5s（T_STALL 3s + T_QUIET 2s + DA1 往返）；stdin 非 tty（重定向）时 crossterm 私有 `/dev/tty` 无法置非阻塞、看门狗观察受限；stdin 指向非控制终端的 tty 时输入源会切换到 `/dev/tty`。

### 缺陷修复（Bug Fixes）

- 增量批次保留 `MailboxStatus::Parsing` 总量（计划 `statusbar-gauge-spinner`）：`Account::process_event` 的增量 `Fetch` 分支此前每批都会把运行中 total 清零（`Parsing(prev_len + len, 0)`），导致任何依赖 `MailboxStatus` 的 `LineGauge` 在第一批之后无法恢复进度。现在把 `done` 与 `total` 同时读出，与累计的 `done` 一起原样写回，并附注释说明 total 为何是承重字段。

- 修复启动时版本迁移提示误判：`${XDG_DATA_HOME}/meli/.version` 中残留的非 semver 值（如 `meli-git`）此前按字符串字典序被当作"更新的版本"，导致每次启动都弹出降级警告与 CAUTION 交互询问。现在该值会被诚实报告为一行 warning，按"早于全部已知版本"处理（经 `is_applicable` 预检后照常提供适用迁移），并在运行结束时写回当前版本，一次启动即自愈。
- 版本比较改为 semver 数值比较：修复 `0.10.0` 等合法版本因字典序（`'1' < '9'`）被误判为旧版本的问题；pre-release（如 `0.8.8-rc1`）现按 semver 先行级排在同版本号正式版之前，其迁移定位不再回退为全量迁移。
- thread-view 垂直滚动键（`Up`/`Down`）在正文聚焦态到界停住：连按 Right 聚焦正文（MailView）后，垂直滚动将正文滚到顶/底即停住（no-op 消费按键），不再冒泡驱动会话列表光标、也不再自动切换到上一封/下一封邮件；分屏默认态（focus=None）下 `Down` 仍直接移动会话列表光标，既有行为不变。注意退化：若将 thread-view 垂直键与 pager 垂直键改绑为不同键，正文聚焦态下 thread-view 垂直键为纯停住（既不滚正文也不动列表）。

<!-- ### 重构（Refactoring） -->

<!-- ### 文档（Documentation） -->

<!-- ### 打包（Packaging） -->

### 杂项（Miscellaneous Tasks）

- 核对确认与上游 meli 完全同步（截至 2026-09-15）：上游 HEAD `3d7eb2c5` 自 2026-09-14 同步 `4f2414a3..3d7eb2c5` 以来无新提交（见 [SYNC.zh-CN.md](./SYNC.zh-CN.md)）。
- 与上游 meli 完全同步（截至 2026-09-28）：上游区间 `3d7eb2c5..bb6d5916`（22 个提交）在四个并行 worktree 上语义移植——PGP 可插拔后端、JMAP EventSource 推送及八个修复/重构提交（逐提交记账见 [SYNC.zh-CN.md](./SYNC.zh-CN.md)）。刻意跳过：命令补全框架重构 `a041bc90`（fork 自有命令面板 + nucleo 模糊匹配已覆盖并更优）、`ListingTrait::select` 纯内部重构 `03e1f5de`（无外部调用方）、上游 CI 移除 cargo-derivefmt 的 `0eaae124`（fork CI 为绿且保留该步骤）、`quote` 升级 `45a5d376`（fork 已在 1.0.47）。已通过 `make check`、`make lint`、`make test`（全 feature 全绿）。


## [v0.9.0] - 2026-09-13

加固版（hardened）meli fork 的首个版本。汇总自仓库创建（2026-09-08，基于上游
meli v0.8.13）以来的全部 fork 改动，安全加固与用户体验工作优先列出。

### 安全加固（要点）

- 修复 18 项安全审计发现（3 项高危）：mailto/撰写路径的 CRLF 头注入、mailcap
  %-替换注入与 fnmatch 条件反转、SMTP 短行 panic 与 UTF-8 未定义行为、无上限
  的服务器响应（现以 64 MiB 封顶并加 SMTP 读超时）、JMAP 跨域重定向凭据泄漏、
  multipart 解析器挂起/panic（嵌套上限 + 边界循环防护）、vCard 字符边界 panic。
- HTML 邮件默认清洗：fork 内置基于 ammonia 的清洗器（`meli_sanitize_html`），
  接入 `pager.html_filter`，随 meli 一起构建安装，附带 golden 与 CLI 一致性测试。
- 凭据绝不写入 trace 或错误信息：SASL/IMAP trace 脱敏、`password_command` 输出
  不再随错误外泄、`meli.log` 以 0600 权限创建。
- 打开非 http(s)/mailto 的 URL 与跟随 List-Unsubscribe 链接前先弹确认提示；
  桌面文件 Exec 解析支持引号。
- IMAP 接收路径加固：UID FETCH 回复缺 UID 项按协议错误处理而非 panic；
  tag-not-last 帧、裸 `+` keepalive 行、pre-continuation 推送与 IDLE 中途的
  `* BYE` 均已处理。
- 构建链：UCD 表经 HTTPS 拉取并锁定 SHA-256。

### 用户体验（要点）

- 新邮件真正自动到达——历史最大痛点：INBOX 保持选中时刷新可能漏信（过期的
  STATUS 快照，已用 NOOP 验证），不主动推送的服务器（如 QQ 邮箱）上 IDLE 会话
  会永远等待。被监视邮箱现在仅由一条专用只读 IDLE 连接持有（单会话不变量），
  并以心跳重同步兜底（间隔默认 60 秒）——新邮件一分钟内到达，零配置。
- 快速且离线可恢复的启动：网络重同步前先供应缓存信封（stale-while-revalidate）、
  邮箱列表与 MSN 索引持久化在 sqlite3、离线取信优雅降级、重连时对账邮箱列表；
  QQ 规模的首次拉取大幅提速。
- 侧栏 `Right` 打开选中邮箱；thread-view 新增 `focus_left`/`focus_right` 窗格
  快捷键、快捷键遮蔽修复，会话选择移动时正文实时跟随。
- `C-s` 保存全部附件（`save-all-attachments`）；`Up`/`Down` 方向键成为全局
  默认滚动键。
- 面向真实服务器的健壮解析：ENVELOPE 字段拆成多个 token 或以原始非 NSTRING
  字节发送均被容忍（QQ 邮箱兼容），并附文档化的 QQ 邮箱示例账号模板。

### 可靠性与正确性

- 移植 8 个上游修复/性能提交（错误分类、SELECT 复用、文本 content-type 守卫、
  响应缓冲、缓存重 SELECT 规避、TagsIterator、tag 间距、`tags.rename`）。
- SMTP 回复码 353 与 PRDR 解析；RFC 4549 STATUS 快速检查；完整接收路径 trace
  收敛在 `debug-tracing` 之后（能力字节、每连接入站计数、ID 交换），用于诊断
  服务器怪癖；新增 `watch_sweep_interval` 账户设置。
- 代码库评审清理：删除约 1500 行死代码，去重 mailcap/mbox/thread-view 逻辑，
  同步 man 手册与文档注释。
- 版本迁移框架：修正上游的排序比较器为真正的 semver 排序（原逐分量 `>=` 会
  拒绝任何 minor 升级）。

### 测试与质量

- 封闭（hermetic）XDG 测试环境与去全局化的 `MELI_CONFIG` mock（消除并行竞态）；
  `make test` 全绿无跳过。
- 带场景模式的 mock IMAP 服务器（多会话推送抑制、ID 门控推送、过期 STATUS
  快照、粘包写入），每个修复都有先红后绿的回归钉，并用真实抓取的推送字节构建
  重放夹具；`scripts/test.sh` 作为唯一测试入口。

## 上游历史版本（v0.8.13 及更早）

本仓库基于上游 meli v0.8.13 创建；v0.8.13 及更早的版本条目为上游原始发布
记录（含逐提交链接，英文），刻意保持原文不做翻译，请直接查阅英文版
[CHANGELOG.md](./CHANGELOG.md) 中 `[v0.8.13] - 2026-01-04` 起的历史段落。

[Unreleased]: #
