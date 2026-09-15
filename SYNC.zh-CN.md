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
