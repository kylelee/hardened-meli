[English](./SYNC.md) | **简体中文**

# SYNC.md — meli 上游同步记录

本仓库是 [meli](https://github.com/meli/meli) 的下游 fork，我们会持续同步上游代码。
本文件记录每次同步上游（upstream）的时间和内容，便于追溯每个本地改动对应的上游提交。

## 上游仓库

| 仓库 | 地址 |
| --- | --- |
| 上游（GitHub） | <https://github.com/meli/meli> |

建议本地配置：

```sh
git remote add upstream https://github.com/meli/meli.git
git fetch upstream
```

## 记录格式

每次同步完成后，在下方「同步记录」**顶部**追加一节（新记录在前）：

```markdown
## YYYY-MM-DD HH:mm (UTC+8)

- 同步方式：<逐提交语义移植 / merge / cherry-pick>
- 上游范围：<起始哈希>..<结束哈希>（或逐条列出）
- 合并提交：<本地 merge/commit 哈希>
- 冲突处理：<无 / 冲突文件及取舍说明>
- 记账类型：条目可为 SKIP（fork 已有等价物或不适用移植，附原因）、PARTIAL（移植了指名 hunk，其余延后——必须登记债务）或 N:1（N 个上游提交合入一个本地提交，附原因，如上游 broken-intermediate 提交）

| 本地提交 | 上游提交 | 类型 | 说明 |
| --- | --- | --- | --- |
| <hash> | <hash> | fix/perf/feat/refactor | <一句话说明> |
```

## 同步记录


## 2026-10-12 23:30 (UTC+8)

- 同步方式：八个并行 worktree 分支（`sync/contacts-framework`、`sync/jmap-core`、`sync/backend-trait`、`sync/pgp-sequoia`、`sync/melib-fixes`、`sync/carddav-backend`、`sync/jmap-contacts`、`sync/frontend-integration`）按依赖分两波语义移植，各由 dsh 子代理执行；UI 层提交按策略刻意跳过（fork UI 架构更先进）
- 上游区间：`aea4508b..168410ab`（73 个提交；53 个移植——其中 5 个部分移植、丢弃 UI hunk——20 个跳过，见下表）
- 合并提交：`a0ddb2a1`（melib-fixes）、`c2dcd560`（jmap-core）、`42585c11`（contacts-framework）、`f41fd7b1`（backend-trait）、`f36305f3`（pgp-sequoia）、`96945e7d`（carddav-backend）、`a5ffd126`（jmap-contacts）、`ce6f4ab6`（frontend-integration）
- 合并期语义解冲突：
  - `melib-test/tests/jmap/main.rs`：S3a（上游 `5b68048d` 移植：keywords 跟踪 + tracing + `StateChange` 广播 + `EmailObjRef` 寻址）对撞 S4（为 save-with-flags-and-tags 测试独立重建了同套 mock 机制并新增 `POST /upload`/`Email/import`/真实 `Email/query`）。裁决：保留 S3a 骨架 + S4 的 upload/import/query 增量；两套分歧的 `keywords` 存储统一为 JMAP 原生 `IndexMap<String, bool>` 形态并加 `flag_keyword` 辅助函数（flag 位关键字经 `Email/get` 可回环）；冗余的 `set_flags_by_id`/`set_flags(String)` 并入 `set_flags(EmailObjRef, FlagOp)`。
  - `melib/src/imap/mod.rs`：S1 的字面量 `supports_contacts: false, extra_submission_headers: &[]` 被 S4 的 `..MailBackendCapabilities::default()` 展开包含（Default = EMPTY 常量，值相同）。
  - `melib/src/utils/mod.rs`：S4 与 S6 各加了一份 `canonicalize_crlf`，`fba25257` 去重。
- 验收：`make check`、`make lint`、`make test` 全绿（全 feature）；MSRV 1.85.0 → 1.88.0（上游 `edaa6736`）。

| 本地提交 | 上游提交 | 类型 | 说明 |
| --- | --- | --- | --- |
| `d4593170` | `b0c6bcda` | fix(melib) | SMTP `ReplyCode` 解析 `530` |
| `1576386c` | `9cc5293d` | fix(melib) | `Error::set_source` 尝试 downcast 回 Self（fork 保留 `Arc`；同提交的 Arc→Box 重构未随迁） |
| `1ca89a1d` | `c83c9e67` | fix(melib) | 拼接 summary/details 不重复句号 |
| `27cde630` | `f1bd593d` | fix(melib) | Display 始终展示 related path |
| `e4c9155d` | `d6b2bc79` | fix(meli) | 临时文件冲突重命名保留扩展名（`rsplit_once`，优于上游按最后点重切） |
| `c8a69b20` | `834679d6` | refactor(melib) | JMAP capabilities 迁独立模块 |
| `3ccbc1bf` | `085de222` | refactor(melib) | identities 存 Store 而非 Session |
| `87ba174d` | `c1b32e3a` | perf(melib) | mail `account_id` 缓存进 EmailFetcher |
| `862d2907` | `4325dbe1` | fix(melib) | Set 响应只含 server-set 属性 |
| `7762737e` | `d5ba7be8` | fix(melib) | EventSource 流免超时 + 60s 连接超时 |
| `66f2f10e` | `5b68048d` | test | 测试服务器跟踪 keywords（其 Email/import+upload 设施由 S4 的 `eeb91753` 移植补齐，后者同时带来 `EmailImport` wire 类型） |
| `210b5313` | `3e7133b9`+`62c24b52`+`db9f7f93`+`6c61f34b` | feat(melib) | AddressBook 多容器 + Contacts 门面（手写 serde 保持旧单书 JSON 持久化形状；fork 前端经兼容门面零改动编译） |
| `656cb283` | （同上，meli 侧 hunk） | refactor(meli) | 仅最小机械适配（accounts.rs 3 行） |
| `ed273b6a` | `62c24b52`+`6c61f34b`+`5eda1ee1` | feat(melib) | `ContactBackend` trait + Capabilities + CRUD；`MailBackendCapabilities::supports_contacts` 与 `MailBackend::contact_backend()` 默认方法 |
| `f76b947d` | `1554983e` | feat(melib) | mutt alias 文件后端 |
| `912fe2d3` | `982dd328` | feat(melib) | notmuch 地址查询后端 |
| `8a3007e0` | `254d8a5a`+`6c61f34b`+`d771b08f` | feat(melib) | vCard 文件后端 + 值折叠检测（保留 fork 的 CWE-754 单文件隔离语义） |
| `7dfd8720` | `8b82457a` | refactor(melib) | Card 的 `Display` 渲染为 Address 形态 |
| `7da07e3d` | `d5ecc5a8` | feat(melib) | notmuch 查询 `--output=recipients` + 手册 |
| `15bfc9c3`/`5f8a7110`/`68e20787`/`b8331cbd`/`6641e214`/`54e3e033` | `06a63d1b`/`195aabb6`/`d08d69e7`/`b61d1178`/`3a98bf00`/`c5b0faaa` | feat/fix(melib) | JSContact Clone、Send/Sync、schema 修复、serde 实现、name components、API 修正 |
| `39a123b4` | `d628a7dd` | fix(melib) | NNTP 提交路径 CRLF 规范化（免 regex 探测） |
| `eda853ee` | `9e0d441d` | refactor(melib) | imap capabilities 改 `..default()` 展开 |
| `96c0fd6a` | `4dd81ffa` | refactor(melib) | `submit()` 删除从未使用的 flags 参数 |
| `b1d4cf9c` | `eeb91753` | feat(melib) | `save` 同时携带 flags 与 tags（imap APPEND keyword / jmap import 后 EmailSet keywords / maildir·mbox·nntp 签名涟漪） |
| `07a617e8` | `cb633c63` | feat(melib) | notmuch 仅刷新指定 mailbox |
| `7b626f2b` | `b2360adf` | feat(melib) | notmuch 数据库 revision（无变化跳过刷新） |
| `6f5f2633` | `fa4a3daa` | feat(melib) | notmuch `Message` Debug 实现 |
| `458a3680` | `7cf9e3c2` + `924d00f8` | feat(melib) | notmuch `save_to` + `MailboxConf` extra 任意值（ExtraSetting） |
| `ff2e2d34` | `2fa32889` + `6e0740db` | feat(melib) | Sequoia PGP 后端——opt-in cargo feature `sequoia`（默认关闭）、PGPBackendChoice::Sequoia、decrypt 返回 SignaturesMetadata；fork 的 CLI 后端保留先签后加密（不回退） |
| `dec0b9fe` | `e8276c7a` | fix(melib) | gpgme 加载失败回退备选库名 |
| `a3a8055e` | `b0b2aa1e` | perf(melib) | `convert_attachment_to_rfc_spec` 返回 Cow |
| `cdd0d637` | `d4d83770` | refactor(melib) | canonicalize_crlf 迁 utils |
| `b8a6a558` | `edaa6736` | chore | MSRV 1.88.0 |
| `9a9310a8` | `16ada4cb` + `6af4a32e` | ci | Gitea CI：sequoia 构建依赖 + 显式 `--features sequoia` 校验步骤 |
| `e889d0fe` | `88c8451c` + `6c61f34b` | feat(melib) | WebDAV 客户端 + CardDAV ContactBackend（isahc 2、http 1.x、quick-xml；`webdav` feature 镜照上游 default 集） |
| `f67b51ee` | `14640216`（melib 侧） | feat(melib) | `AccountSettings::carddav_server_conf`（conf 键 `carddav`） |
| `45a9611b` | `281da6a3` + `6c61f34b` + `c5b0faaa` | feat(melib) | JMAP 通讯录后端（ContactCard CRUD、capabilities 接线、`contact_backend()` 覆写） |
| `24756e5d` | — | fix(melib-test) | jmap_redirect Store 字面量补 `identities`（第一波涟漪） |
| `9a569183` | — | test | 离线 jmap-contacts 用例 + mock 服务器 contacts 能力 |
| `97cce6a1` | `db9f7f93`（melib conf） | feat(melib) | `ContactBackendConf` 枚举 + 每账户 `contacts` map；删除旧 `notmuch_address_book_query`/`mutt_alias_file`/`vcard_folder` 键 |
| `05911e9e` | `6c61f34b` | refactor(melib) | 默认书后端标识对齐上游 `DefaultContacts` |
| `3f29255a` | `db9f7f93` + `6c61f34b` + `14640216` | feat(meli) | accounts/contacts.rs 翻译到 fork 的 tokio 作业执行器；后端装载（mutt/vcard/notmuch/CardDAV/JMAP）+ CRUD 作业；fork 默认书 JSON 持久化与 `external_resource` 过滤语义保留（往返单测锁定） |
| `4ed6dbe6` | `db9f7f93`（迁移） | feat(meli) | `v0_12_0` ContactBackendRefactor 迁移（旧三键 → `contacts` map）+ 工作区版本 0.12.0 |
| `5e6f987f` | `ab506b45` | fix(meli) | ServerPasswordCommand 迁移补链 related path |
| `0db9af49` | `c3de4afd` | feat(meli) | `meli migration` 工具（list/apply/revert，dry-run/verbose） |
| `ce4654bf` | `c6c49d77` | docs(meli.1) | 补缺失 CLI 参数 |
| `38803c54` | `db9f7f93` + `14640216`（docs） | docs(meli.conf.5) | `contacts` 配置面 + CardDAV 段 |
| `fba25257`、`73b2b127`、验收修复批 | — | fix | 合并残留：canonicalize_crlf 去重；cve 夹具适配 `Contacts::cards()` 门面与 `Store.identities`；clippy/文档验收批；`decl_version_map` doctest 示例 map 扩至 `v0_12_0`；CVE 语料断言面适配（2386 换 `contacts` 配置 + 跨书 `search("")` 计数、1088 重锚 accounts/contacts.rs 与 add_card 计数、0145 折行改等价性断言、2009 族 danger_* 白名单纳入 webdav.rs）、SMTP 530 往返测试更新 |

  合并前已覆盖（无单独移植）：`dc14fbbf`（job metadata——被 `3f29255a` 的翻译吸收）、`c8f0a95e`（`From<bool> for IsAsync`——同形实现随 `3f29255a` 落地）、`924d00f8`（随 `458a3680`）。

  跳过（20 个提交，UI/主题/清理类、落在分叉形态上）：

| 上游提交 | 理由 |
| --- | --- |
| `3e7133b9`/`db9f7f93`/`6c61f34b`/`14640216`（仅 UI hunk） | contacts list/editor/export UI 重做——fork UI 更先进；经 Contacts 门面 fork UI 零改动编译 |
| `15ec10ec`、`168410ab` | contact 导出对话框 + Escape 取消——UI |
| `21895e71`、`c9182179`、`0cc149ee`、`82d3a4b5` | 通用组件（Enter 消费、normal mode、secondary_area、List 组件）——UI |
| `1c9cbe30`、`e6b2bdfe` | 任意输入长度补全——fork 撰写页补全已更先进 |
| `fc9ab541`、`3fa4ec87`、`b5d389db` | contacts list/editor 行为修复——UI |
| `64d89b8e` | MailboxManager 迁通用 List——UI |
| `a0dc6d37`、`5d41a2bc` | OSC10/11 自动主题 + themes 模块公开——UI/主题 |
| `082f3c58` | 内嵌终端 SGR 分组——UI 性能 |
| `33147fad`、`e4098bf9`、`5aa4fb7a`、`dcf153ee` | 分叉形态上的琐碎修复/清理 |
| `d3640f72` | workspace `[lints]` 段——fork CI 的 `RUSTFLAGS=-D warnings`（含 tests/examples/benches）已承担同等门禁；照搬上游 nursery/style 全量 deny 会拖入 fork 分叉 UI 代码内的数十处 clippy 修正 |

## 2026-10-03 01:20 (UTC+8)

- 同步方式：两个并行 worktree 分支语义移植（`sync-melib-trio`、`sync-flag-toggle`），各由一个 dsh 子代理执行,每项改动 TDD 先红后绿
- 上游区间：`253ba7dd..aea4508b`（16 个提交；4 个移植、12 个 SKIP——见下表）
- 合并提交：`361f97b`（sync-melib-trio,fast-forward）、`aea11df` 经合并 `86d1ede`（sync-flag-toggle）
- 冲突：无（文件集不相交）
- 移植缺口披露：上游 `f6ddf9a4`（2026-09-28,`deserialize_extra_field` + `ExtraSetting` 类型化 extra 反序列化）早于本区间且未被 2026-10-01 移植——该缺口正是数值/布尔 extra 配置被静默吞掉 bug 的根因,已由本地 `ebdfcff8`（`AccountSettings::extra_conf_string`）修复。已知分歧：fork 保留 `extra_conf_string` 字符串强转（已对 QQ 邮箱 993 隐式 TLS 实网验证）;上游走类型化反序列化。后续任务：评估统一为单一机制。
- 验证：`make check`、`make lint`、`make test` 全绿;CI 对等：`Makefile.build`（含 rustdoc）+ `Makefile.manifest-lint`（cargo-sort + debian/changelog）+ `clippy` 绿——`rustfmt` 与 `cargo-msrv` 本机跳过（无 nightly 工具链 / cargo-msrv 未安装且 ~/.cargo 只读;CI runner 具备）;保护门槛：`test_account_settings_extra_conf_string` 与 `test_conf_numeric_and_boolean_extra_values_reach_imap_server_conf` 原样通过、四处 `get_conf_val!` 宏未动;实网冒烟：qq + work 993 隐式 TLS 登录成功（`M2 OK Success login ok`）、0 条 STARTTLS 行

| 本地提交 | 上游提交 | 类别 | 类型 | 说明 |
| --- | --- | --- | --- | --- |
| `361f97b` | `5151e75c` | port | fix(melib) | `set_flags` 忽略 `Flag::PASSED`（Set/UnSet 空操作臂）——此前落入「more than one flag bit」错误分支;mock-server 测试 `test_imap_set_flags_ignores_passed`（修复前验证为红） |
| `361f97b` | `32733460` | port | perf(melib) | `BackendEvent` 手写 `Debug`：`RefreshBatch` ≥30 条只打印条数+前 30（派生 Debug 全量打印导致日志行数 MB 级膨胀）;单测验证输出有界与阈值下完整 |
| `361f97b` | `1218cb74` | port | feat(melib) | `TryFrom<Vec<EnvelopeHash>> for EnvelopeHashBatch`（空 vec → `Err`）;单测覆盖空/单/多元素 |
| `aea11df` | `8404d74a` | port+deviation | feat(meli) | `flag toggle <FLAG>` 命令（上游 #765）：`FlagAction::Toggle`、解析器第三 alt 臂、双批执行（无标志→Set 批、有标志→UnSet 批）合一个 `toggle-flag` job;偏差：解析臂按 fork 惯例、集合转换用既有 slice `TryFrom`（不依赖 Vec impl）、spawn 走 `account.is_async()`;命令面板补全表已登记 |
| — | `aea4508b`、`7d1d5b4d`、`4fb09060`、`b690d871` | **SKIP** | feat(meli) | 附件编辑 Add/Remove 按钮与 ButtonWidget 主题——UI 层,按策略不同步 UI（fork 自有 UI 架构） |
| — | `90e68695`、`b7b7565e` | **SKIP** | chore | 上游分叉代码形态上的 clippy 修正;fork CI 本就全绿 |
| — | `537c687e`、`68f368aa`、`cf64e366`、`553f3baf` | **SKIP** | feat(meli) | 命令补全改进——被 fork 的 nucleo 命令面板取代 |
| — | `a41cb7b0`、`08561c25` | **SKIP** | feat(melib) | `ShellExpandTrait::expand_tilde` 与补全波浪号展开——仅服务上游补全;fork 无消费方（视图路径已用 `expand()`）,移植即死代码 |

## 2026-10-01 21:30 (UTC+8)

- 同步方式：七个并行 worktree 分支语义移植（批次一：`t1-mailcap`、`t3-notmuch`、`t4-melib-fixes`、`t5-trace-flag`、`t6-listing-compose`、`t7-contrib-gpg`；批次一合并后批次二：`t2-secret`）
- 上游区间：`bb6d5916..253ba7dd`（35 个提交；6 个 SKIP/部分跳过——见下表）
- 合并提交：`96c35621`（t7）、`c7e0745c`（t1）、`9cfbcd80`（t6）、`eb44b9ac`（t4）、`bf84926b`（t3）、`1b568f27`（t5）、`0fcd631d`（t2）
- 冲突：仅 `meli/src/mail/listing/conversations.rs` 的 `filter()`——t4 的 `get_threads` Option 化与 t6 的计数元组精简（限制移除）合并
- 合并期验收修复：`e5c9b3c0` + `ac9dc1a7`（clippy 1.98 新增规则合规）、`9a33ac65`（退订测试对齐异步 `send_draft_async` job 路径——旧断言为 `5a04bc0d` 上的存量问题，已在 detached 基线复现）、`92f06575`（extra 字段由 `serde_json::Map` + `preserve_order` 特性改为 `IndexMap`——该特性悄悄改变了 JMAP 线上键序并破坏 `test_jmap_query`；上游本就用 `IndexMap` 且不开该特性）、`decl_version_map` 文档示例补 `v0_10_0` 条目
- 验证：`make check`、`make lint`、`make test` 全绿（全 feature，0 告警）

| 本地提交 | 上游提交 | 类别 | 类型 | 说明 |
| --- | --- | --- | --- | --- |
| `705de33c` | `0d4b0bf9` | port+deviation | refactor(meli) | `ProcessRequest` 结构体 + `temporary_files`；fork 让文件存活到 `result_cb` 之后（上游通配绑定在进程运行前就 drop） |
| `f8c11e21` | `c8cad6fb` | port | fix(meli) | `spawn: None` 分支补退出码检查；`spawn: Some` 分支 fork 本就有 |
| `31f87da4` | `253ba7dd` | port+deviation | feat(meli) | RFC 1524 完整引擎（全字段、`MailcapParser`、`run_candidates`、上游测试套）；重放 fork 加固：全部替换值 POSIX 单引号 shell 引用、未知 `%` 序列返回 `Error`（上游 panic）、nametemplate 越界修正、坏条目逐条跳过（上游跳整个 label） |
| `e61a8b1f` | `2309c167` | port | fix(meli) | `sanitize_filename` 标点清洗收窄为 `!"'/\`；补边界测试 |
| `3646287b` | `2ca62c90` | port | refactor(melib) | `Drop` 移到 `DbPointer`（持 `Arc<NotmuchLibrary>`），`DbConnection` 可 `Clone`；顺带消除 debug_assert expect panic 路径 |
| `035223be` | `05a08b6c` | port+deviation | fix(melib) | refresh 对比新旧 tags/存在性发精确 `RefreshEvent`；`LazyCountSet` 计数；偏差：`mailboxes.get()+continue` 替代 map 索引 panic、set_flags/NotmuchOp 用 let-else 返错 |
| `f3f8fa42` | `0d7e1532` | port+deviation | refactor(melib) | imap+notmuch 单锁 `MailboxCounters`；fork 的 cache-first 分页、ghost 清理、watch 补偿保留；jmap/maildir/nntp 双锁站点未迁移（见债务） |
| `b612ca4c` | `57e60bec` | port | fix(melib) | notmuch 搜索词 `AND` 组合（上游 #766） |
| `4ba5328e` | `09c6d05c` | port | perf(melib) | 抓取分块 250→1000 |
| `934b8838` | `3a19fe2e` | port | refactor(melib) | `ignore_not_found` 提为 `melib::error` 公共函数 |
| `dacdd4b3` | `ee38e475` | port+deviation | perf(meli) | `AccountCache::update`/rename 原地更新；insert/update 共享私有 `store()`；保留 fork `account_id` 加固，`?` 链替代上游 `unwrap` |
| `4695f051` | `d45ea5fe` | port+deviation | fix(melib) | `get_mailbox`/`get_threads` Option 化（fork `get_env` 已是）；上游 compose/view 签名重构不需要（fork 已适配） |
| `23d930f2` | `34e40e0e` | port+deviation | fix(melib) | maildir 用户操作直发事件；保留「缓存锁下不做文件系统 IO」纪律与并发修改对账 |
| `8715f27a` | `6429fccc`+`d75d3be8` | **2:1** port+deviation | feat(conf) | 账户级 `trace` 替代四个 -trace 特性；fork 保留 `debug-tracing`（./log/）与 `to_str!`；连接 id 日志沿用 fork 方案；`test_trace_redact_*` 脱敏套件保持绿 |
| `4f4153b1` | `e4565617` | port | fix(meli) | Trash 不再回退 Junk |
| `ca826d98` | `2b86929b` | port | fix(meli) | 过滤态再搜索查全邮箱（fork 四种 listing 同病）；回归测试 `filter_on_top_of_filter_searches_whole_mailbox` 红绿验证 |
| `c39d9313` | `2d7fa2fa` | port+deviation | feat(command) | `public-inbox import`/`import-thread`（lore.kernel.org）；fork parser 字节流签名、确认经 `UIEvent::Callback`（fork 对话框 done_fn 无 context 参数）、补全表+文档+parser 测试 |
| `492f72eb` | `7fe6cc1e`+`0ef78a0d`+`78eb0d5e`+`bb17a5bc` | **4:1** port | feat(compose) | 多地址补全（解析合法前缀、仅补末段、去重）、`Contacts::search` 返 `Card`、`From<Card> for Address`、fields 子模块与 `test_compose_address_complete` |
| `ab257c5a` | `a7c98b05`+`547f600e` | **2:1** port+deviation | fix(contrib) | Python 3.9 兼容 + 错误 JSON stderr；偏差：修复上游 `hash_algo = str(other)` 的 NameError bug（改 `str(hash_algo)`） |
| `d7fb255a` | `f6ddf9a4` | port | feat(melib) | `ExtraSetting` trait（+`Secret::prepopulate`） |
| `a0ced034` | `97a08539` | port+deviation | feat(conf) | extra 值化；验收修复 `92f06575` 定型为 `IndexMap<String, serde_json::Value>`（上游形态）——`serde_json::Map` + `preserve_order` 曾破坏 JMAP 线上键序 |
| `ecba109b` | `254cee97` | port+deviation | feat(melib) | 全部 server 个人字段 `Secret` 化、认证字节组装前最后刻求值；fork 的密码命令错误卫生迁入 `Secret::value`；oauth2 等价校验；`server_password_command` 校验期拒绝并提示迁移 |
| `b9434a12` | `483f0629` | port+deviation | feat(meli) | `ServerPasswordCommand` 迁移挂新 `v0_10_0`（fork `v0_9_0` 已发布且与上游内容不同）；`is_applicable` 用 `get_included_configs` + raw contains；crate 版本升至 0.10.0 |
| — | `266b918a` | **SKIP** | refactor(ui) | Selector 回调传 context——fork 对话框已重写（UI） |
| — | `59ffaaeb` | **SKIP** | refactor | RowsState 去泛型——纯内部重构，fork listing 已重写 |
| — | `facc045c` | **SKIP** | refactor | 删 `to_str!`——fork 容错 IMAP 解析器仍在用 |
| — | `9ff2e38e` | **SKIP** | fix | `change_log_level` 设 max level——fork 已有（带注释） |
| — | `41b547c6` | **SKIP** | test | mock 配置 TRACE——fork 测试基建不同 |
| — | `63894a9c` | **SKIP** | test | `new_mock` 环境重置——fork 每测试自有 hermetic XDG 助手 |
| — | `05dde1d2` | **部分 SKIP** | chore | 过时宏部分由 `d75d3be8` 移植覆盖；fork 保留 `debug-tracing` cargo 特性（./log/ 落盘）——文档已更新为双层说明 |

- 债务登记（承接 + 新增）：
  - FilterOutputMetadata 未移植（解密收件人/逐过滤器签名状态展示）——在 1-2 个同步周期内对齐 fork 的 SignedVerified 管线 *(自 2026-09-14)*
  - filters.rs ViewFilter 路径不触发明文验证 *(自 2026-09-14)*
  - imap 测试基建分叉：上游 imap 测试提交需重实现 *(自 2026-09-14)*
  - sqlite3 搜索后端忽略 raw_search（上游怪癖，原样保留）*(自 2026-09-14)*
  - 上游命令补全框架未移植；关注后续上游 `meli/src/command/**` 提交中的非 UI 修复 *(自 2026-09-28)*
  - `mailbox_changed` SSE 分支已实现但 mock 服务器未覆盖 *(自 2026-09-28)*
  - jmap/maildir/nntp 双锁计数未迁移到 `MailboxCounters`（上游 `0d7e1532` 也改了它们；fork 为控制本轮影响面仅做 backends+imap+notmuch）*(新增 2026-10-01)*
  - `test_jmap_watch` 的 Destroy 断言曾让手动 `refresh()` 与 SSE 触发的 resync 争抢同一状态差分（drain 早于事件送达）；已改为与 Create/NewFlags 断言一致的 await watch future 模式——竞态已消除，此条留作溯源 *(新增 2026-10-01，当日解决)*

## 2026-09-28 03:47 (UTC+8)

- 同步方式：四个并行 worktree 分支语义移植（`t1-melib-conf-fixes` melib/conf 小修复、`t2-pgp-backends` PGP 后端、`t3-jmap-eventsource` JMAP 推送、`t4-compose-notify-fixes` compose/notify 修复）
- 上游区间：`3d7eb2c5..bb6d5916`（22 个提交；4 个 SKIP——见下表）
- 合并提交：`b6360713`（T1）、`5f0c3d3a`（T3）、`240a1cbf`（T2）、`57490df6`（T4）
- 冲突处理：仅 `meli/src/conf/tests.rs`——双方都在文件尾追加新测试块，两者皆保留（T1 的 `test_conf_tag_rename` + T2 的 `pgp_backend_choice_tests`）
- 验证：`make check`、`make lint`、`make test` 全绿（全 feature，0 警告）

| 本地提交 | 上游提交 | 记账 | 类型 | 说明 |
| --- | --- | --- | --- | --- |
| `3e49d1a1` | `ed162e11` | port+deviation | fix(melib) | `resync_condstore` 仅在 condstore FLAGS 处 `== 0`→`< 2`；`resync_basic` 为闭区间 `..=`，经核对无空区间缺陷，不动（提交正文已注明） |
| `8fab4a01` | `3f8427b0` | port | fix(melib) | PUA → Ambiguous 宽度 `Some(1)` + `wcswidth("\u{F09B}")` 回归测试 |
| `8eda294c` | `d8cc16b9` | port | fix(meli) | `TagName` 哈希 `TagHash`（非 name）；字段 `pub`；新增 `test_conf_tag_rename` |
| `53b101f3` | `fe48ab20` | port | refactor(meli) | fork 等价位置移除 feature 相关导入 |
| `c975e9a4` | `b33e50fc` | port | style(melib) | 修正 clippy `allow` 属性位置 |
| — | `9cbb4f41` | **SKIP** | style | fork 的 `conf.rs` 无该 import（顶层仅 `extern crate serde;`） |
| `d0250dc6` | `40a45b04` | port+deviation | refactor(melib) | 从 `methods.rs` 抽出 `url_template` 模块（RFC 8620 URI 模板，含单测）；fork 保留 `Arc<FutureMutex<usize>>` + async `add_call`（不采纳上游 `AtomicUsize` 重写，纯内部实现） |
| `98fd3296` | `af619461` | port+deviation | feat(melib) | 对 `eventSourceUrl` 的 EventSource(SSE) 推送取代轮询；偏差：协议违规由 `panic!`/`assert_eq!` 改为 `ErrorKind::ProtocolError`（fork 不 panic 加固）；SSE 请求 `RedirectPolicy::None`（延续 fork 跨源重定向凭据加固，上游 `Limit(10)`）；删除死字段 `last_method_response` 与 `new()` 硬编码 10s 超时（否则会掐断 `timeout: None` 的 SSE）；fork 测试插桩（`error_responses`、since_state==current → 空响应）保留；新增 `run_jmap_watch`/`test_jmap_watch` SSE mock 用例 |
| `afbd67f8`+`debe90c0`+`5805c091` | `89f834b6`+`97f02477`+`7110e8d0` | **3:1** port+deviation | feat(pgp) | `melib::email::pgp` 新增 `PGPBackend` trait + `Key` 抽象；gpgme 收敛为 trait 实现；`cli` 脚本后端（`[pgp] backend = "cli"`）+ `contrib/pgp-cli-backends/gpg/` 六个 GnuPG 参考脚本；`compose/gpg.rs`→`pgp.rs`；keylist `IndexSet` 去重；反序列化错误提示。偏差：cleartext 管线（`UnverifiedSignature`/`extract_unverified_signature`/`verify_cleartext`）与 SignedPending→SignedVerified 路由逐字保留，不引入 ViewFilter/FilterOutputMetadata；`PGPBackendInstance` 为 owned 以适配 `JobExecutor::spawn`；用 `From<gpgme::Key> for pgp::Key` 替代上游 `GpgmeKey` 重命名；上游在 `command/{actions,parser}.rs` 移除 `#[cfg(feature="gpgme")]` 守卫的改动**不移植**（保 fork no-gpgme UI 契约，延续 `00d3b65d` 语义）；`overrides.rs` 经 sentinel 重新生成 |
| `971f1f9d` | `59c5ad4b` | port | fix(compose) | 编辑器解析：`composing.editor_command` > `$VISUAL` > `$EDITOR`，提示文案更新 |
| `86348503` | `bb6d5916` | port | fix(compose) | 编辑器参数经 argv 传递（`sh -c '<editor> "$@"'` + `.arg(&editor).arg(path)`），杜绝拼接；fork 的 `EDITOR_TEMP_FILENAME_MAX_BYTES` 加固不动 |
| `d9c32af8` | `c498d7e6` | port+deviation | fix(ui) | 画框前清完整 `cached_area`（含边框）；适配 fork 的 `draw_rounded_frame`（fork 无 `create_box`） |
| `dc0537a5` | `6d3dd4bd` | port | fix(ui) | compact 标签文本从 `area_col_4.skip_cols(1)` 起打印 |
| — | `a041bc90` | **SKIP** | feat(ui) | 上游命令补全框架重构（新 `completions.rs`、parser/UI 重做，约 2800 行）；fork 命令面板 + nucleo 模糊匹配已覆盖并更优（用户决策 2026-09-28）。债务：后续涉及 `meli/src/command/**` 的上游提交须对照 fork 解析器人工评估，不能直接 diff |
| — | `03e1f5de` | **SKIP** | refactor | `ListingTrait::select` 提升——纯内部重构，整个上游区间内零外部调用方；fork 保留各 listing 固有实现 |
| — | `0eaae124` | **SKIP** | ci | 上游移除 cargo-derivefmt（对新 Rust 语法失效）；fork CI 为绿且保留该步骤——仅在 fork 的 derivefmt 也开始失败时再评估 |
| — | `45a5d376` | **SKIP** | chore(deps) | fork `Cargo.lock` 的 `quote` 已在 1.0.47 |

- 债务登记：
  - FilterOutputMetadata 未移植（解密收件人 / 每过滤器签名状态展示）——在 1-2 个同步周期内对齐 fork 的 SignedVerified 管线 *（承接 2026-09-14）*
  - filters.rs ViewFilter 路径不触发 cleartext 验证（经 ViewFilter 打开的邮件绕过 envelope.rs 路由）*（承接 2026-09-14）*
  - imap 测试基建 fork 化：上游 imap 测试提交须重实现（见 SKIP b08a39b3）*（承接 2026-09-14）*
  - sqlite3 搜索后端忽略 raw_search（上游怪癖，原样保留）*（承接 2026-09-14）*
  - 上游命令补全框架未移植（fork 命令面板已覆盖）；关注后续上游 `meli/src/command/**` 提交中夹带的非 UI 修复
  - `mailbox_changed` SSE 分支已实现但 mock 服务器未覆盖（上游新套件同样未覆盖）

## 2026-09-15 02:11 (UTC+8)

- 同步方式：仅核对（无代码变更）
- 上游范围：`3d7eb2c5`（上游 HEAD 无变化——自 2026-09-14 23:05 同步后零新提交）
- 合并提交：无（仅文档）
- 冲突处理：无

| 本地提交 | 上游提交 | 记账 | 类型 | 说明 |
| --- | --- | --- | --- | --- |
| — | — | 核对 | docs | 上游镜像 `git fetch` 后确认 `origin/master` 仍为 `3d7eb2c5`（2026-09-14 12:44:51 +0300）；fork 已完全同步。CHANGELOG.md `[Unreleased]` 记录该状态。债务登记不变（2026-09-14 23:05 的 4 项未偿还） |

## 2026-09-14 23:05 (UTC+8)

- 同步方式：两个并行 worktree 分支语义移植（`sync-24h-a` 搜索链、`sync-24h-b` PGP 链）
- 上游范围：`4f2414a3..3d7eb2c5`（19 个提交；排除 WIP `8b51d601`）
- 合并提交：`956abda0`（链A）、`7d183160`（链B）
- 冲突处理：无（两链无文件交集）

| 本地提交 | 上游提交 | 记账 | 类型 | 说明 |
| --- | --- | --- | --- | --- |
| `a3432d8f` | `d1941d6f` | 移植 | chore(deps) | futures manifest → 0.3.34（Cargo.lock 早已解析） |
| `68ef3e66` | `2e00c62e` | 移植+偏差 | ci | nextest ci profile；**terminate-after=4（上游 2）**：fork watch 测试自限 30s（`WATCH_TEST_DEADLINE`），40s 保留先 panic 后 kill 的诊断顺序；`Makefile.build` 接线 `--profile ci --config-file` |
| `25a2a9f8` | `3d7eb2c5` | 移植 | docs | BUILD.md 删 MSRV 句（Cargo.toml 为唯一事实源） |
| `5f54195c` + `de8e6c1b` | `d321e3f2` | 移植+偏差 | test(jmap) | since_state==当前态回空响应而非 cannotCalculateChanges；**防御性对齐**（现有场景该分支不可达，非行为证明）；fork 自加 `error_responses` 服务端插桩（`de8e6c1b` 为对抗评审发现的去重/补第二站点修复） |
| `0fac10a5` | `2c5670cb` | 移植 | test(notmuch) | macos 库检测（`library_file_path`） |
| — | `b08a39b3` | **SKIP** | 测试基建 | imap 测试服务端重构（2243 行）：fork 自有 8700 行套件覆盖更强（含 fetch 缓存回归）；上游新套件无 raw_search 覆盖。债务：后续上游 imap 测试提交需重实现而非搬运 |
| — | `939c17d5` | **SKIP** | test | test_imap_fetch 依赖 b08a39b3 基建；fork 有自有 fetch 缓存测试（melib/tests/imap/main.rs:199-249） |
| `8e280430` | `9e7014cf` | 移植 | refactor(melib) | EMPTY_MAIL_BACKEND_CAPABILITIES + Default，6 处构造收敛 |
| `0fcafcb0` | `75752b38`+`c121b79e` | **2:1** | feat(melib) | 上游 `75752b38` 单独不可编译（imap capabilities 全字面量缺新字段——broken intermediate）；supports_raw_search + `raw_search` trait 默认方法（NotSupported）+ Gmail X-GM-EXT-1 检测 + UID SEARCH X-GM-RAW literal。fork 自有：mock 续行臂、`run_imap_raw_search_gmail`、`test_maildir_raw_search_not_supported`、预存在默认特性测试编译修复（解除 `set_test_xdg_env`/`fetch_all_envs` gate、19 处 `cfg!` offline_cache；base 上 39 个 E0425——随本提交而非独立 prep 提交，在此注明） |
| `4c83d5aa` | `2825d224` | 移植 | feat(notmuch) | raw_search 直通（mailbox query_str 前缀拼接）+ 测试；本机无 notmuch 二进制 skip，CI 装 notmuch 真跑 |
| `32590b9d` | `a072648f` | 移植 | feat(command) | raw-search/raw-select 命令、Search/Select 元组→struct 变体（9 个 listing arm 等）、man +15 行；**sqlite3 搜索后端忽略 raw_search 的上游怪癖逐字保留**（上游 HEAD 未修）；fork 自有：parser 单测 + account.search 接线判别测试 |
| `8ef3a7c0` | `ad3252b0` | 移植 | ui | ascii_drawing 下 StatusBar ascii mouse 回退 |
| `00d3b65d` | `ab62f82a` | 移植 | fix(compose) | toggle 相关 gpgme feature guard |
| `29987b9c` | `d2c91b59` | 移植 | feat(pgp) | cleartext 验证：`UnverifiedSignature` + `extract_unverified_signature` + `Context::verify_cleartext` + 安全警告；上游测试逐字移植（测试密钥三方 md5 一致）；fork 自有：melib 三分支单测；CI apt 加 `gnupg libgpgme11` |
| `0d2e1d7a` | `d5e9360c` | **PARTIAL** | feat(view) | 仅非 view hunk（mail.rs 解 gate + pgp.rs 函数级 cfg，与上游逐字节一致）+ fork 路由：text/plain cleartext → 既有 SignedPending/SignedVerified 管线，显示原始 armored 文本不剥壳；filters.rs 未动。未移植：ViewFilter/FilterOutputMetadata 机制（view 层已被 ratatui 波次重写）——见债务 |
| `81037c3d` | `1b837d42` | 移植 | refactor(view) | 删 `EnvelopeView::html_filter` 死字段 |
| `46bba4bb` | `0cad2282` | 移植 | refactor(melib) | LocateKey 由 gpgme 迁至 email::pgp；overrides.rs 经 sentinel 再生成 |
| `77d12462` | — | fork 自有 | test(melib) | extract_unverified_signature Detached 分支负路径测试（对抗评审发现：原 happy-path 测试在 micalg 校验被删时仍绿） |

- 债务登记：
  - FilterOutputMetadata 未移植（解密收件人/逐过滤器签名态显示）——1-2 轮内对齐 fork 自研 SignedVerified 管线
  - filters.rs ViewFilter 路径不触发 cleartext 验证（经 ViewFilter 打开的邮件绕过 envelope.rs 路由）
  - imap 测试基建分叉：上游 imap 测试提交需重实现（见 SKIP b08a39b3）
  - sqlite3 搜索后端忽略 raw_search（上游怪癖，逐字保留）


- 同步方式：语义移植（semantic port）
- 上游提交：`4f2414a3`
- 合并提交：`05d8b13c`（branch `meli-upstream-sync-24h`，翻译自上游 cache-then-resync 排序变更）

| 本地提交 | 上游提交 | 类型 | 说明 |
| --- | --- | --- | --- |
| `c250f34c` | `4f2414a3` | fix(melib) | IMAP 同步改为先从缓存 fetch 再 resync，用服务器真相对账集合（cache-then-resync ordering） |

## 2026-09-11 22:52 (UTC+8)

- 同步方式：分支批量移植（branch `sync-upstream-meli`，基于 `e9f480c`）
- 合并提交：`71093054` — 共 8 个上游修复/性能提交
- 冲突处理：`meli/src/mail/listing.rs` 唯一冲突文件，双侧保留 —— main 的 `#[cfg(test)] mod listing_menu_tests` 原位保留，同步分支的 `TagsIterator`（struct + impls）追加在其后；`focus_right` 侧边栏 hunk（~:2198）自动合并保留 main 版本
- 后续修正：`c05f976e`（2026-09-12）— 将测试模块移到 `TagsIterator` 之后，修复 stable-rustfmt 漂移

| 本地提交 | 上游提交 | 类型 | 说明 |
| --- | --- | --- | --- |
| `4eb6f8b1` | `2b7c2123` | fix(melib) | 错误分类：先按 `ErrorKind` 归类再回退裸 errno |
| `8d076703` | `48490fcc` | perf(melib) | `examine_mailbox()` 接受已存在的 SELECT，复用连接状态 |
| `b8f8ec91` | `4ca86e2a` | fix(melib) | `get_text_recursive()` 检查 text content_type |
| `d5af6d3e` | `40cab0f7` | perf(melib) | 新增 `FetchState::response` 缓冲区（语义移植） |
| `16846e42` | `18654e06` | perf(melib) | FetchState 缓存阶段避免强制 re-SELECT（语义移植） |
| `d86129b1` | `5863123b` | refactor(meli) | 新增 `TagsIterator`（语义移植） |
| `219a85d0` | `1713ec06` | fix(meli) | 打印标签时预留 1 格空间 |
| `f52f1fdb` | `31298d3f` | feat(meli) | 新增 `tags.rename` 设置项 |
