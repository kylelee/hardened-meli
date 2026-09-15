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

- 输入线程私有 CSI 看门狗：检测「字节被 crossterm 消费但无事件产生」的卡死（如 mux/终端迟到的 `CSI ?` 私有序列回复在 crossterm 0.29 解析器内无限缓冲吞键），3s 停滞 + 2s 输入静默判定后自动注入 DA1 查询（`ESC[c`）触发终端回复使解析器整包清缓冲，恢复后续按键（被吞按键不可恢复）；连续 3 次注入无恢复则进程内停用；正常使用零注入。输入循环主等待重构为 `poll(2)`，输入线程将 stdin 换为非阻塞 tty 描述以避免 crossterm 内部读取死锁。新增 `scripts/test-private-csi-watchdog.sh` PTY 回归门（tmux 缺失时 SKIP）。

- Composer 新增关闭快捷键（`[shortcuts.composing]` 的 `close` 键，默认 `Esc`）：在邮件编辑界面按 `Esc` 关闭标签页返回之前的界面，草稿有未保存修改时弹出 x/y/n 对话框（不保存退出 / 保存草稿退出 / 取消）；附件管理模式按 `Esc` 返回主编辑界面。同步补齐移除附件、附件内层保存、收件人确认、gpg 密钥确认四处 `has_changes` 缺口，避免 Esc 关闭时静默丢弃未保存修改（计划 `composer-esc-exit`）。

### 变更（Changes）

- 依赖治理：移除根 `Cargo.toml` 的 `[patch.crates-io]` 与 `vendor/crossterm/` 目录，crossterm 回归 crates.io 0.29.0 原版。原补丁防御的「未识别私有 CSI 卡死」改由删除无用启动查询（`CSI ? 2026 $ p` 同步输出支持探测，全代码库无消费者）+ 输入看门狗承担。离线构建改为依赖本地 cargo cache 预取（`cargo fetch`）。

### 已知问题（Known Issues）

- 看门狗已知限制（与 vendor 补丁时代的非回归项一致或为新增残留）：`<`（SGR 鼠标半截序列）卡死不受 DA1 注入保护；迟到 OSC 10/11 回复产生的垃圾键为正常事件、看门狗对其失明；mux 以 <2s 间隔持续注入 `CSI ?` 回复时看门狗不触发；极慢链路大粘贴理论上有 DA1 字节混入风险；卡死恢复延迟最坏 ≈5s（T_STALL 3s + T_QUIET 2s + DA1 往返）；stdin 非 tty（重定向）时 crossterm 私有 `/dev/tty` 无法置非阻塞、看门狗观察受限；stdin 指向非控制终端的 tty 时输入源会切换到 `/dev/tty`。

### 缺陷修复（Bug Fixes）

- 修复启动时版本迁移提示误判：`${XDG_DATA_HOME}/meli/.version` 中残留的非 semver 值（如 `meli-git`）此前按字符串字典序被当作"更新的版本"，导致每次启动都弹出降级警告与 CAUTION 交互询问。现在该值会被诚实报告为一行 warning，按"早于全部已知版本"处理（经 `is_applicable` 预检后照常提供适用迁移），并在运行结束时写回当前版本，一次启动即自愈。
- 版本比较改为 semver 数值比较：修复 `0.10.0` 等合法版本因字典序（`'1' < '9'`）被误判为旧版本的问题；pre-release（如 `0.8.8-rc1`）现按 semver 先行级排在同版本号正式版之前，其迁移定位不再回退为全量迁移。
- thread-view 垂直滚动键（`Up`/`Down`）在正文聚焦态到界停住：连按 Right 聚焦正文（MailView）后，垂直滚动将正文滚到顶/底即停住（no-op 消费按键），不再冒泡驱动会话列表光标、也不再自动切换到上一封/下一封邮件；分屏默认态（focus=None）下 `Down` 仍直接移动会话列表光标，既有行为不变。注意退化：若将 thread-view 垂直键与 pager 垂直键改绑为不同键，正文聚焦态下 thread-view 垂直键为纯停住（既不滚正文也不动列表）。

<!-- ### 重构（Refactoring） -->

<!-- ### 文档（Documentation） -->

<!-- ### 打包（Packaging） -->

### 杂项（Miscellaneous Tasks）

- 核对确认与上游 meli 完全同步（截至 2026-09-15）：上游 HEAD `3d7eb2c5` 自 2026-09-14 同步 `4f2414a3..3d7eb2c5` 以来无新提交（见 [SYNC.zh-CN.md](./SYNC.zh-CN.md)）。

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
