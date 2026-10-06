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

- CVE-2023-35619 Outlook for Mac UI 欺骗攻击模拟语料（issue #134）：2023-12 补丁日公告（CVSS 5.3，CWE-451），MSRC 未公开根因——`UI:R` 是唯一硬约束，映射为头部显示真实性等价面。`cve/src/CVE-2023-35619.rs` 携带真实欺骗语料（`From` 显示名以裸 UTF-8 与 RFC2047 编码字两种形态携带 `U+202E`、`Reply-To`/`Sender` 与 `From` 地址分叉、显示 host 与 `href` host 不一致的 HTML 锚点），锁定核查结论：暴露并修复 2 处真实缺口，1 面免疫证明。缺口 A：`sanitize_display_name` 原本只清 ASCII `C0` 控制字符，Unicode Format 类不可见字符（bidi 覆盖 `U+202A`–`U+202E`、孤立 `U+2066`–`U+2069`、零宽/方向标记、`BOM`、软连字符）可直达全部头部显示名——清理范围扩展为 `Cc`（除 `HTAB`）+ `Cf` 标记全家族，公开为 `melib::email::strip_spoofing_invisibles`，并在 `EnvelopeView` 绘制点对 From/To/Cc 三个地址行统一应用（覆盖地址列表解析失败时的原始头值回退路径）。缺口 B：回复实际走向 `Mail-Reply-To`/`Reply-To` 优先于 `From`，但分叉面从不显示——头部带现在把与 `From` 不一致的 `Sender`/`Mail-Reply-To`/`Reply-To` 直接画在 `From` 正下方（值经 `Address` 重解析获得同等清理；已被 `show_extra_headers` 覆盖的面跳过），渲染级孪生单测在 `meli/src/mail/view/tests.rs`。链接脚注面免疫证明：`html2text` 把真实 `href` 渲染为脚注与伪装锚文本并排可见，提取/可启动集合恰为可见集合。

- CVE-2024-50624 KMail 明文 autoconfig MITM 攻击模拟语料（issue #124）：KMail < 6.2.0 的 `ispdbservice.cpp` 通过明文 HTTP 拉取邮件服务器配置（Mozilla autoconfig XML），中间人可返回攻击者控制的 IMAP/SMTP 端点集。meli 没有 autoconfig/ISPDB/autodiscover 功能面，唯一「从网络下发服务器端点」的功能面是 JMAP 会话发现，语料据此映射。本次侦察检出并修复两个真实缺口：`JmapServerConf::new` 与 `JmapType::validate_config` 接受远程 `http://` 的 `server_url`（会话发现 GET 本身即在明文链路上携带 Basic/Bearer 凭据），且 `JmapConnection::connect` 对会话对象返回的 `apiUrl`/`uploadUrl`/`downloadUrl`/`eventSourceUrl` 无任何 scheme 校验即使用。现两者都强制 `https`（明文 `http` 仅限环回地址如 127.0.0.1/localhost，保留 melib-test 环回 mock 与本地开发可用），并在启动期校验与会话解析期 fail-closed 拒绝；`meli/docs/samples/sample-config.toml` 记录该规则。`cve/src/CVE-2024-50624.rs` 分四层锁定。
- CVE 调研外部清单核对合并（issue #6，第二批）：将外部 KIMI 调研清单中经逐条核对的 50 个新增 CVE 编号合并进 `cve/SECURITY-CVE-RESEARCH.zh-CN.md`（含英文版）——表 1 新增解密预言机回复泄露家族 CVE-2019-10732/10734/10735/10740/10741（KMail/Trojitá/Claws Mail/Roundcube/K-9 Mail 同族五连，合并为一行）与 KMail 远程内容绕过 CVE-2018-19516；表 2 新增 Outlook CVE-2024-38173/2023-35636/2023-36763/2023-35619/2000-0567/2001-0145、mutt 2018 批次 CVE-2018-14349–14363（15 个整合为一行，替换原三编号行）、Sylpheed 系 CVE-2003-0300/2005-0667、Roundcube 反序列化 RCE CVE-2025-49113、Evolution CVE-2005-2549/2550/2008-1108；表 3 新增 Outlook/OE 经典 CVE-2000-0621/2001-0999/2001-1088、KMail CVE-2020-11880、Roundcube 在野利用群 CVE-2020-35730/2023-43770/2023-5631/2024-42008/2021-44026、Zimbra CVE-2024-27443/2025-27915/2025-66376、Horde IMP CVE-2025-30349；表 4 新增 FORCEDENTRY CVE-2021-30860 与 WebKit CVE-2021-30761；表 5 新增 SigSpoof 2 CVE-2018-12019 与 NO STARTTLS 系列 CVE-2021-29969/2016-10727/2020-28896/2020-15047/2021-38372/2021-39272。全部编号经 NVD API 2.0 核实编号存在、描述与受影响产品相符，CVSS 缺失者按 NVD 值补齐并标注版本；去重：CVE-2018-0950/2021-31855/2018-12020/2024-42009 沿用既有行，表 5 的 CVE-2019-10732 单行被表 1 家族行取代，CVE-2024-42008 单独成行不与 42009 合并；未收录：CVE-2021-32066（Net::IMAP 库）仅在趋势中述及、CVE-2019-1073x/1074x 为家族速写。趋势观察新增第 8–11 条（webmail XSS 国家级武器化、STARTTLS 系统性失败、签名/加密语义缺口、Mailsploit 无 CVE 盲区），第 8 节对 meli 的启示新增 6 条（ICS/可外发认证引用白名单、协议响应字段 fuzz、TLS fail-closed、gpg 状态输出防注入与多签名、回复只引用 MIME 树根明文、写回服务器保持密文），并新增第 10 节增补记录；中英两版逐表同步。
- 按 issue #6 模板为第二批 78 个 CVE（外部 KIMI 清单核对合并 50 个 + 查缺补漏 28 个）创建攻击模拟任务 issue #101–#178，并更新 `cve/README.md` 看板索引。
- CVE 调研查缺补漏（issue #6）：为 `cve/SECURITY-CVE-RESEARCH.zh-CN.md`（含英文版）五张主题表补充 23 条经逐条联网核实的条目（涉及 28 个 CVE 编号）——追踪与隐私（Office RTF 邮件 OLE 信息泄露 CVE-2018-0950、Thunderbird OpenPGP 私钥明文落盘 CVE-2021-29956、KMail 解密明文回传服务器 CVE-2021-31855）、恶意代码执行（Outlook RCE CVE-2020-16947；2018 年 mutt `imap/command.c`/`imap/util.c` 系列 CVE-2018-14349/14351/14352 与 `imap_qresync` 越界读 CVE-2021-32055；Evolution Data Server base64 整数溢出 CVE-2009-0587；KMail 附件名溢出 CVE-2000-0481）、网页嵌入（KMail 纯文本查看器 HTML 注入与 QWebEngine JS 执行 CVE-2016-7966/7967/7968、RainLoop 查看器 XSS CVE-2022-29360、Roundcube 反清洗/CSS 过滤绕过 XSS CVE-2024-42009/42010、SquirrelMail `magicHTML`/`javascript:` XSS CVE-2002-2086/1649、Zimbra Classic UI `<img>` 标签 XSS CVE-2024-45516）、邮件可达解码器（ImageMagick ImageTragick CVE-2016-3714）与协议/信任边界（GnuPG `--status-fd` 文件名伪造 CVE-2018-12020 及 KMail CVE-2007-1265；KMail UI 显示加密实走明文 POP3 CVE-2020-15954、STARTTLS 未生效 CVE-2021-38373、明文 autoconfig MITM CVE-2024-50624、Send Later/自动加密遗漏 CVE-2017-9604/2014-8878、隐藏密文回复泄露 CVE-2019-10732）。每条均经 CIRCL Vulnerability-Lookup API 与 NVD 2.0 API 核实编号存在性、描述、受影响产品及 CVSS 分值与版本；核实后判定超范围、重复或误归属的候选（服务端接口 XSS、非邮件途径的共享备忘、同族重复、误归属为 Vim 漏洞的 CVE-2023-4735）予以剔除。中英两版逐表同步。
- CVE-2008-4491 加密邮件草稿明文落盘回归语料（issue #17）：Apple Mail 3.5 在开启“草稿存储在服务器”后，把 S/MIME 加密往来的草稿以明文保存在服务器上，机密对话对邮件服务器管理员与链路中间人完全可读。`cve/src/CVE-2008-4491.rs` 携带真实 PGP/MIME 语料——gpg 2.4.9 cv25519 逐字密文（生成时解密回环验证，测试断言 OpenPGP 包头），内容为业务上可信的汇款指令邮件——并把攻击映射到 meli 的撰写草稿生命周期：线上原文与 melib 解析都不暴露会话明文；引用存储密文不泄露任何内容；而引用解密后的会话视图正是把明文写进草稿序列化的那一步——并记录：阻止这些字节静默进入服务器 Drafts 邮箱的闸门是撰写器的保存策略，由下述修复的 meli 回归锁定（草稿从不自动保存；发送启动失败拒绝持久化已武装加密的草稿；显式保存给出明文警告；发送后的存储副本是已加密的线上原文）。
- CVE-2008-3068 加密回执信标回归语料（issue #16）：Outlook/Windows Live Mail/Office 2007 在 CryptoAPI 吊销检查时自动访问 S/MIME 证书内嵌的 AIA/CRL 分发点 URL，每次读信都向攻击者泄露阅读时间与 IP。`cve/src/CVE-2008-3068.rs` 携带一封密码学上真实有效的 openssl 签名 S/MIME 语料（内嵌证书向 `attacker.example` 宣告 OCSP、CA Issuers 与 CRL 分发点三类信标 URL）以及协议走私变体，逐层证明 meli 等价面的免疫性：CMS 部件解析为不透明 `CMSSignature` 数据块、信标 URL 不出现在任何邮件元数据；唯一的签名验证入口拒绝非 OpenPGP 协议，走私载体也只把不透明字节交给引擎；且每个 gpgme 上下文创建即离线/仅本地/不自动取钥（GPGME 离线模式：CMS 禁 Dirmngr CRL/OCSP、OpenPGP 彻底禁用 Dirmngr）——证书内嵌 URL 永远不会被访问。
- MFSA-2005-11 Cookie 追踪回归语料，`cve` crate 首个逐 CVE 回归（issue #13）：该通告（Thunderbird 0.6–0.9 / Mozilla Suite 1.7–1.7.3）中 HTML 邮件内嵌内容发出带 Cookie 的 HTTP 请求（`<img>` 追踪像素、样式表 `<link>`、CSS `@import`/`url()`），无视“邮件中禁用 Cookie”偏好，垃圾邮件可借此追踪收件人。`cve/src/MFSA-2005-11.rs`（经 `cve/src/lib.rs` 以 `#[cfg(test)] #[path]` 挂载）含 20 个自动加载信标向量与一封完整追踪垃圾邮件，从结构上证明 meli 的免疫性：每个向量在 `sanitize` 后标签与 URL 一并消失（由独立的标签/属性白名单 oracle 校验）、渲染输出为纯终端文本、仅存的远程引用是用户可见且 meli 绝不主动抓取的链接脚注、`sanitize` 对语料为不动点（无二次解析重组）。`meli::mail::view::html_render::{sanitize, render}` 由 `pub(crate)` 改为 `pub`，使 `cve` crate（如 meli-test 一样）测试的就是邮件视图实际调用的加固路径本尊；渲染管线仍是纯内存文本处理、无抓取器也无 Cookie 存储，未发现 meli 防御缺口。
- 逐 CVE 攻击模拟任务 issue（issue #6）：按调研报告为每个 CVE（含 MFSA-2005-11）各建一个 Gitea issue，共 78 个；每个 issue 含 CVE 背景、映射到的 meli 攻击面（HTML 清理器、MIME/IMAP/SMTP 解析、mailcap、gpgme、TLS/STARTTLS）、攻击样例构造要点、预期断言与 `cve/src/<cve-id>.rs` 回归语料验收标准；索引见 `cve/README.md`。CVE-2025-66376 已由 issue #5 的语料覆盖，不重复建单。
- 新增 `cve` workspace crate（issue #8）：建立脚手架（`Cargo.toml`、`src/`、`README.md`），承载针对 `meli`/`melib` 的 CVE 驱动回归测试，避免给发布 crate 添加测试专用依赖；根目录 CVE 调研报告 `SECURITY-CVE-RESEARCH.md`（含中文版）移入 `cve/`，根 README 双语链接同步更新。
- 邮件客户端/浏览器 CVE 调研报告（issue #6）：新增 `SECURITY-CVE-RESEARCH.zh-CN.md`（含英文版）——从公开 CVE 库中筛选出以电子邮件为攻击载体或直接攻击邮件客户端的漏洞，按五张主题表整理：追踪与隐私（追踪像素、Cookie 信标、S/MIME AIA 信标、EFAIL、CSS 渗出）、恶意代码执行（1999 年 mutt/Eudora 到 2026 年 Thunderbird 的 MIME 解析内存破坏、Outlook 零点击 NTLM 凭据链、iOS Mail 零点击）、网页嵌入（webmail SVG/附件预览 XSS、URI 处理器与 `file://`/`mhtml:` 滥用）、邮件可达的浏览器引擎（WebKit/libwebp/BLASTPASS）、协议与信任边界（恶意 IMAP 服务器、STARTTLS、签名覆盖范围），并附趋势观察与对照 meli 自身清理器和解析器的加固要点。
- 内置 HTML 清理器新增 CVE-2025-66376 标签切分（tag-splitting）回归语料（issue #5）：Zimbra 的客户端清理器对 HTML *字符串*剥离 `@import` 序列与注释后，把改写结果交回浏览器重新解析，碎片因此重组出可执行的 `<svg onload=eval(atob(…))>` 标签（Proofpoint TA488）。meli 用 html5ever 恰好一次解析、过滤树、带转义再序列化——不存在可供碎片重组的二次解析，输出交给 html2text 渲染纯终端文本而非 JS 引擎。语料（报告原文 exploit 串、三个碎片配方、`display:none` 载体、同族解析差异型 mXSS 经典样本）由惰性 oracle（标签/属性白名单扫描 + `sanitize` 幂等不动点）与渲染不 panic 检查锁定。
- `flag toggle <FLAG>` 命令（上游 `8404d74a`,修上游 #765）:在选中邮件上切换标志——从 collection 读取每封当前 flags,无该标志的收进 Set 批、有的收进 UnSet 批,合并为一个后台 `toggle-flag` job;已登记命令面板补全表;解析臂按 fork 惯例,doctest 覆盖合法/非法标志名与参数数量错误。

- `TryFrom<Vec<EnvelopeHash>> for EnvelopeHashBatch`（上游 `1218cb74`）:空 vec 返回 `Err`,非空拆为 `first`/`rest`。

- mailcap RFC 1524 完整实现（上游 `253ba7dd` + `0d4b0bf9` + `c8cad6fb` + `2309c167`，语义移植；修上游 #556）：`MailcapEntry` 解析全部 RFC 字段（`compose`/`composetyped`/`print`/`edit`/`test=`、`copiousoutput`、`needsterminal`、`nametemplate`、`textualnewlines`），展开 `%s`/`%t`/`%n`/`%F`/`%{param}`（含 multipart `%F` 子部件文件展开），执行 `test=` 探测程序，并管理处理器生命周期：`UIEvent::ProcessRequest` 改为携带 `temporary_files` 的结构体（`Arc<File>` 句柄存活至结果回调结束再释放以清理临时文件），`spawn: None` 路径同样检查非零退出/信号终止。fork 偏差：所有替换值（`%t`、`%{param}`、路径）经 fork 的 POSIX 单引号 shell 引用而非上游裸拼接；未知 `%` 序列返回 `Error` 而非 panic；坏条目逐条跳过。`sanitize_filename` 标点清洗收窄为 `!"'/\`，`@` 与点号在附件文件名中保留。

- 运行时账户级协议跟踪（上游 `6429fccc` + `d75d3be8`）：账户设置 `trace = true`（IMAP/JMAP/NNTP 的 `extra`、SMTP `send_mail` 表）即可开启协议级连接转储，无需重编译——`{imap,jmap,nntp,smtp}-trace` 四个 cargo 特性移除。fork 自有 `debug-tracing` 特性不变（管 `./log/` 落盘）；melib 残留 `debug!` 宏删除、调用迁移到 `log::debug!`/`log::trace!`，`to_str!` 保留（QQ-Mail 容错 IMAP 解析器在用）。trace 凭据脱敏（`test_trace_redact_*`）保留并扩展。

- 全部 server 个人配置字段接受 `Secret`（上游 `f6ddf9a4` + `97a08539` + `254cee97` + `483f0629`；修上游 #448）：`server_username`、`server_hostname`、`server_password`、`server_url`（按后端）接受字符串或 `{ command = "..." }`——由此支持如 `server_username_command`。账户 `extra` 字段反序列化为 `IndexMap<String, serde_json::Value>`（任意 TOML 值、保留配置文件顺序；不启用 serde_json `preserve_order`，JMAP 线上键序不变）。明文仅在认证字节组装前最后一刻求值，绝不进 trace/日志/错误信息；fork 加固过的密码命令执行（错误不泄 stdout）移入 `Secret::value`。**破坏性变更**：`server_password_command` 在校验期即拒绝并指向新语法；新增 `v0.10.0` 版本迁移（`ServerPasswordCommand`）自动改写存量配置，crate 版本升至 `0.10.0`。

- `public-inbox import` / `public-inbox import-thread` 命令（上游 `2d7fa2fa`）：按 Message-ID 从 lore.kernel.org 拉取单封邮件或整线程导入账户邮箱（保存前确认对话框）；已登记进命令面板补全表。

- 撰写页地址自动补全支持多地址（上游 `78eb0d5e` + `7fe6cc1e` + `0ef78a0d` + `bb17a5bc`）：已合法的地址前缀先解析（`email::parser::address::mailbox`），仅对正在输入的末段做补全，过滤已录入地址，结果携带已输入前缀。`Contacts::search` 返回 `Card`，`Card` 可转换为 `Address`。

- `sqlite3::AccountCache::update`（上游 `ee38e475`）：单封邮件的索引更新（如 flag 变更）原地 UPDATE，不再删除+重插。

- `MailboxCounters`（上游 `0d7e1532`）：IMAP 与 notmuch 邮箱计数合并为单互斥锁（`{unseen, total: LazyCountSet}`），消除 unseen/total 锁序死锁一类问题；fork 的 cache-first 抓取分页在其上保留。



- 状态栏提示文字按布局场景化（计划 `statusbar-layout-hints`）：场景敏感提示（`Scroll Up` / `Scroll Down` / `Focus Left` / `Focus Right` / `Search`）改为按细化后的 `Component::hint_focus()`（`Sidebar`、`NoView`、`GridSingleMail`、`GridThreads`、`ThreadList`、`MailView`）查标签表，同一按键按持键盘窗格显示对应动作文字：layout1 侧栏为 `Folder Up` / `Folder Down` / `Focus Maillist`，layout1 网格为 `Maillist Up` / `Maillist Down` / `Open Mail`，layout2/layout3 网格为 `Maillist Up` / `Maillist Down` / `Focus Box` 加 `Focus Content`（单邮件视图）或 `Focus Threads`（线程视图），layout4 线程列表为 `Thread Up` / `Thread Down` / `Focus Maillist` / `Focus Content`。邮件详情态（邮件动作）与非 listing 视图（默认标签）保持不变，按键行为亦不变，仅提示文字变化。

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

- 依赖升级到最新（issue #12）：`meli`/`melib` 全部依赖升级至最新版本，共享依赖上提到 `[workspace.dependencies]`（成员清单仅保留差异化条目与各自追加的 feature）。主要迁移：`nom` 7→8（组合子调用改 `Parser::parse`、字节字面量 tag 改切片、字符谓词改 `AsChar` 方法）、`isahc` 1→2（`SslOption`/`ssl_options` 换 `tls::TlsConfig`，启用 `native-tls` + `tls-insecure` feature，DNS 缓存导入路径变化）、`toml` 0.8→1.1 与 `toml_edit` 0.22→0.25（`Deserializer::parse`）、`xdg` 2→3（`BaseDirectories` 构造不再可失败、home 获取器返回 `Option`）、`meli` 构建脚本 `syn` 1→3（`config_macros.rs` 属性改写移植到 `Meta::List`）、`rusqlite` 0.37→0.40（`fallible_uint` 保留 `usize`/`u64` 转换且改为溢出检查）、`imap-codec`/`imap-types` 2.0.0-alpha.6→alpha.9/alpha.7、`nix` 0.30→0.31、`linkify` 0.10→0.11、`ammonia` 4.2、`base64` 0.23、`notify` 8.2。MSRV 上限维持 1.85（`melib`）/ 1.88（`meli`）：`uuid` 固定 `=1.26.1`（1.27 需 rustc 1.89）、`encoding_rs` 固定 `=0.8.35`（0.8.40 需 1.88）、`libloading` 保持 0.8（0.9 需 1.88）、`notify-rust` 保持 `<4.18`（4.18 需 1.89）。
- 日志重构（issue #11）：`meli`/`melib` 日志依赖由 `log` 换为 `tracing`，并删除 `melib` 原 log 模块（`Logger`、自研 `log::Log`/`Subscriber`、XDG 默认目录）——全部代码直接用 `tracing` 打日志（`BackendEvent::Notice` 携带 `tracing::Level`）。启动时一次无参调用 `meli/src/logging.rs::init_log()` 完成全部配置：`tracing-subscriber` pretty 格式写入 `tracing-appender` 按小时滚动的 `./log/meli.<YYYY-MM-DD-HH>` 文件（unix 下保持 `0600` 权限）、后台保留策略只留最近 7 天日志（每小时检查一次）、debug 构建开 `DEBUG` 级别 / release 构建开 `ERROR` 级别——release 还经 `tracing` 的 `release_max_level_error` 把 `ERROR` 以下全部编译剔除（替代 `log` 的 `release_max_level_off`）。`debug-tracing` cargo 特性移除（日志永远编译开启）；`MELI_DEBUG_STDERR=yes` 仍可把每行复制到 stderr；旧配置键 `[logging] log_file`/`maximum_level` 不再生效并在启动时告警；`meli print-log-path` 改为打印 `./log/` 目录。isahc 的 `tracing` 事件现在真正进入订阅器（此前落入 tracing 的空默认分发器）。单元测试覆盖保留策略与 pretty/级别过滤管线；melib 测试套件的 trace 经 `init_test_logging()` 助手走 stderr。

- notmuch 整修（上游 `2ca62c90` + `05a08b6c` + `57e60bec` + `09c6d05c`）：`Drop`（close+destroy）从 `DbConnection` 移到 `DbPointer`，连接可自由克隆；`refresh` 改为对比当前与快照的 tags/存在性并发出精确 `RefreshEvent`（检测 flag 变化、计数精确增减、重建快照索引）；搜索词组合用 `AND` 而非 notmuch 同前缀隐式 `OR`（修上游 #766）；抓取分块 250→1000。

- maildir 用户主动操作（设 flag/删除/改名）完成后直接发出后端事件（上游 `34e40e0e`），不再依赖 notify watcher 观察文件系统；fork 的「缓存锁下不做文件系统 IO」纪律保留。

- `BackendEvent` 日志输出有界化（上游 `32733460`）：派生 `Debug` 会全量打印 `RefreshBatch` 的每个事件，大刷新产生数 MB 日志行；手写 `Debug` 改为批次 ≥30 条时只打印条数+前 30 条，更小批次与其余变体完整打印。



- 依赖治理：移除根 `Cargo.toml` 的 `[patch.crates-io]` 与 `vendor/crossterm/` 目录，crossterm 回归 crates.io 0.29.0 原版。原补丁防御的「未识别私有 CSI 卡死」改由删除无用启动查询（`CSI ? 2026 $ p` 同步输出支持探测，全代码库无消费者）+ 输入看门狗承担。离线构建改为依赖本地 cargo cache 预取（`cargo fetch`）。
- `envelope-view.reply_to_all` 默认键由 `C-g` 改为 `C-a`（`reply` 保持 `r`，`reply_to_author` 保持 `C-r`）；底栏 `Reply All` 提示与邮件视图按键派发随同一配置绑定自动同步。

- 撰写器在发送启动失败时静默以明文持久化已武装加密的草稿（CVE-2008-4491 同类，issue #17）：当加密过滤器栈或发信管道同步失败（如已武装加密却无法从草稿 `From` 头解析 `encrypt-for-self` 身份）时，未发送的草稿恰在用户已武装加密之际以明文存入（可能是服务器侧的）Drafts 邮箱。该回退现在改为把草稿留在打开的撰写标签页并说明为何未保存副本；显式保存（`save-draft`、放弃对话框的保存）仍然允许，但会警告存储副本为明文而该邮件设置为加密；发送后的存储因加密过滤器先于序列化运行、保存的本来就是已加密的线上原文。回归测试位于 `meli/src/mail/compose.rs`（`send_setup_failure_with_encryption_armed_keeps_draft_out_of_drafts`、`explicit_save_draft_with_encryption_armed_warns_plaintext`，以及未武装加密时行为冻结的对照测试）。
### 修复（Fixed）

- layout1 侧栏 PageDown/PageUp（及 `H`/`L`）只在账号间移动高亮（issue #91）：键盘在邮箱侧栏时，`listing.next_page`/`listing.prev_page` 与 `listing.next_account`/`listing.prev_account` 共用同一输入臂，只把 `menu_cursor_pos` 指向下一/上一账号的默认邮箱，却没有执行 Up/Down 行走时的接管逻辑，导致邮件网格仍列着原账号的文件夹（侧栏已显示新账号的 INBOX，网格却原地不动）。该臂现在套用与 Up/Down 行走相同的 `change_account` + `focus_menu` 接管（`cursor_pos = menu_cursor_pos`），网格立即跟随跳转，键盘留在邮箱列表。回归测试：`layout1_menu_page_keys_switch_account_grid`。
- `melib::gpgme::Context::get_flag` 在刚关闭 `auto-key-retrieve` 后仍报告其为开启：libgpgme 对关闭的上下文 flag 返回空 C 字符串（并非 NULL 指针），而该 getter 只判空不判值。现已改为与 `"1"` 比较，并由 CVE-2008-3068 回归在真实 libgpgme 上运行锁定。
- IMAP `set_flags` 忽略 `Flag::PASSED`（上游 `5151e75c`）：IMAP 协议无 PASSED 的线上表示，设置/取消它会落入「more than one flag bit」应用错误分支并让整条 `UID STORE` 失败；现为显式空操作。mock 服务器回归测试 `test_imap_set_flags_ignores_passed`（修复前验证为红，精确复现上游错误）。

- 账户 `extra` 数值/布尔配置在 `serde_json::Value` 迁移后静默回落默认值（`a0cd...` 移植上游 `97a08539`+`254cee97` 引入）：imap/nntp/jmap/mbox 的 `get_conf_val!` 宏只按 `as_str()` 取值，TOML 的 `server_port = 993`、`timeout = 90`、`use_idle = true` 等变成 `Value::Number`/`Value::Bool` 后永远匹配不上。症状：`server_port` 回落到 143，连带 `use_starttls` 默认翻转为 `true`——QQ 邮箱（imap.qq.com / imap.exmail.qq.com，143 端口拒 STARTTLS 回 `* BAD Command!`）连不上，而 163（Coremail 容忍 143 STARTTLS）侥幸能用。新增 `AccountSettings::extra_conf_string` 把 `Number`/`Bool` 标量强转为字符串，恢复旧版全字符串语义；`Value::Object`（Secret 表）仍返回 `None`。回归测试 `test_account_settings_extra_conf_string`、`test_conf_numeric_and_boolean_extra_values_reach_imap_server_conf`（修复前验证为红：端口解析成 143）。

- 过滤态下再次搜索作用于全邮箱（上游 `2b86929b`）：四种 listing 此前都把新搜索结果限制在上一次过滤存活的行集内；新增回归测试 `filter_on_top_of_filter_searches_whole_mailbox`（旧代码上验证为红）。

- 无 Trash 文件夹时不再回退用 Junk（上游 `e4565617`）——Junk 是垃圾邮件专用；「无 Trash 文件夹」提示保留。

- `Collection` 获取器 Option 化（上游 `d45ea5fe`）：`get_mailbox`/`get_threads` 在邮箱被并发移除时不再 panic，账户调用点改为跳过。`ignore_not_found` 提升为 `melib::error` 公共函数（上游 `3a19fe2e`）。

- gpg CLI 后端脚本：Python 3.9 兼容与错误 JSON 中正确的 `stderr`（上游 `a7c98b05` + `547f600e`），并修复上游 `gpg_sign.py` 未知哈希算法分支引用未定义 match 绑定的 bug。

- 退订确认测试对齐异步发送路径（`send_draft_async` 的 job 派发即确定性发送证据，断言恰好一次）——修正退订发送路径收口后遗留的同步通知断言。


- IMAP 离线缓存遍历改为按行分页，消除 163/Coremail 的刷新风暴（状态栏图标常转）：`CacheFirst`/`FromCache` 抓取阶段原先按 UID 空间做 `max_uid -= batch_size` 步进，在长寿服务器的稀疏 UID 空间（`UIDVALIDITY = 1`、`uidnext` 达 10^8-10^9、真实邮件只有少量）下一次抓取要迭代数千个几乎全空的缓存窗口——每个窗口都是一次瞬时 sqlite 查询并发出一个 `MailboxUpdate` payload——状态栏邮箱图标因此永久转个不停（日志证据：四个邮箱整场会话合计约每分钟 5 万个 `MailboxUpdate`，每次抓取约 600 个瞬时 `fetch-mailbox-continued` chunk）。`ImapCache::envelopes` 现按 `ORDER BY uid DESC LIMIT ?` 返回最新 `batch_size` 行并附带本页最低 UID（隔离区占位行随同页窗口一并服务），两个阶段的下一页都推进到 `最低 UID − 1`，页面为空或抵达 UID 1 即结束：无论 UID 空间多稀疏，遍历只需 `O(缓存行数 / batch_size)` 次查询，首次抓取数秒内完成，之后只做增量同步（验收目标：刷新完成后安静，仅剩定时 watch/IDLE）。

- IMAP 虚拟 Junk（隔离区）在服务器双口径不一致时不再整目录误删缓存（issue #179）：腾讯企业邮箱（imap.exmail.qq.com）`SELECT "Junk"` 返回 `* 0 EXISTS`（其 `FETCH`/`UID FETCH`/`SEARCH` 回复亦为空），而 `STATUS "Junk"` 仍回 `MESSAGES 1 ... UIDNEXT 3`。`resync_basic` 的 RFC 4549 第 4 步以「缓存中存在、但 `UID FETCH 1:lastseenuid (FLAGS)` 回复中缺失」推断该邮件已被 expunge，于是空的选中视图让全部缓存 Junk 邮件都被当成已删除：离线缓存整目录被清空、每封各发一个 `Remove` 事件，列表因此永久为空而计数仍显示 `[1 messages]`。现当解析出的 `STATUS` MESSAGES 大于 `select_mailbox(force=true)` 的 `EXISTS`（两口径不一致）时跳过整段删除对账，推迟到下一次 resync；真实 expunge（两口径同步递减）不受影响。回归测试：`test_imap_resync_status_view_disagreement_keeps_cache`（failing-first）与 `test_imap_resync_real_expunge_still_reconciles`。

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

- 与上游 meli 完全同步（截至 **2026-10-02，上游 HEAD `aea4508b`**）：上游区间 `253ba7dd..aea4508b`（16 个提交）在两个并行 worktree 上语义移植——IMAP `Flag::PASSED` 忽略、`BackendEvent::RefreshBatch` 日志有界化、`EnvelopeHashBatch` Vec 转换、`flag toggle` 命令（修上游 #765）；十二个提交刻意跳过（附件编辑按钮与主题属性——UI 层按策略不同步、分叉形态上的 clippy 修正、被 fork nucleo 命令面板取代或不适用的补全与波浪号展开；逐提交记账见 [SYNC.zh-CN.md](./SYNC.zh-CN.md)）。台账同时披露 `f6ddf9a4` 移植缺口：上游类型化 `deserialize_extra_field` extra 值机制（2026-09-28）未随 2026-10-01 移植——正是数值/布尔 extra 配置静默吞掉 bug 的成因，已由本地 `extra_conf_string` 修复；两机制并存为已知分歧，后续另立统一任务。

- 与上游 meli 完全同步（截至 **2026-10-01，上游 HEAD `253ba7dd`**）：上游区间 `bb6d5916..253ba7dd`（35 个提交）在七个并行 worktree 任务上语义移植——mailcap RFC 1524 重写与进程管理、`Secret` 凭据字段与 `v0.10.0` 迁移、运行时 `trace` 账户开关、notmuch refresh/AND 搜索链、maildir 直发事件、`public-inbox import`、撰写页多地址补全及十项小修（逐提交记账见 [SYNC.zh-CN.md](./SYNC.zh-CN.md)）。刻意跳过：`266b918a`（Selector 回调传 context——fork 对话框已重写）、`59ffaaeb`（RowsState 去泛型——纯内部重构，fork listing 已重写）、`facc045c`（删 `to_str!`——fork 容错 IMAP 解析器仍在用）、`9ff2e38e`（`change_log_level` 设 max level——fork 已修）、`41b547c6`/`63894a9c`（mock 上下文 TRACE/环境重置测试基建——fork 自有 hermetic XDG 助手）、`05dde1d2` 部分（fork 保留 `debug-tracing` 特性管 `./log/` 落盘，仅适用其过时宏部分）。已通过 `make check`、`make lint`、`make test`（全 feature 全绿）。

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
