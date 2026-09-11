<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# meli  ![Established, created in 2017](https://img.shields.io/badge/Est.-2017-blue) ![Minimum Supported Rust Version](https://img.shields.io/badge/MSRV-1.85.0-blue) [![GitHub license](https://img.shields.io/github/license/meli/meli)](https://github.com/meli/meli/blob/master/COPYING) [![Crates.io](https://img.shields.io/crates/v/meli)](https://crates.io/crates/meli) [![IRC channel](https://img.shields.io/badge/irc.oftc.net-%23meli-blue)](ircs://irc.oftc.net:6697/%23meli)

[English](./README.md) | **简体中文**

**安全加强和用户体验优化的 meli 版本 —— BSD/Linux/macos 终端邮件客户端，支持多账户与 Maildir / mbox / notmuch / IMAP / JMAP / NNTP (Usenet)。**

基于上游加固（hardened）而来：<https://github.com/meli/meli> <https://gitlab.com/meli-project/meli>

## 亮点介绍

本仓库是 meli 的增强版分支（fork），在原版基础上完成了全面的安全审计、加固（harden）与重构，并针对实际使用体验做了多项优化。三大核心亮点：

### 1. 安全加固：四道防线抵御邮件攻击

对原版 meli 做了全面代码审计与重构，修复 18 项审计发现（含 3 项高危：mailcap 命令注入、mailto CRLF 头注入、RFC2047 显示名回信劫持），并清理约 1500 行死代码、统一重复逻辑。针对畸形/恶意邮件内容（如 QQ 邮箱未转义引号的 Message-ID、空 local-part 的发件地址、夹带恶意脚本的 HTML 正文）设计了四道防线，层层设防：

1. **解析容错**：IMAP ENVELOPE 字段严格解析失败时，自动回退到原始字节兜底解析——任何单个畸形字段都不会中止整个邮箱的拉取；
2. **摄入净化**：地址字段（From/Sender/Reply-To/To/Cc/Bcc）写入缓存前统一校验与归一化，可修复的自动加引号修复并复验，不可修复的替换为安全占位符——新数据永远不会产生"毒行"；
3. **可见隔离**：历史遗留的坏缓存行不再触发整库重置，而是逐行隔离到 `invalid_envelopes` 表，以占位邮件可见呈现（附错误详情头），服务器重取后自动自愈——杜绝"一封毒邮件核弹整个缓存"。
4. **HTML 清洗**：HTML 邮件正文渲染前先经内置的 `meli_sanitize_html` 过滤器清洗——白名单式过滤（与 `meli` 一同构建安装），仅保留安全的结构标签与 http/https/mailto 链接，脚本、样式、事件属性、注释及 `javascript:`/`data:` 等危险 URL 一律剔除，恶意 HTML 邮件再也无法向渲染视图夹带脚本或跟踪链接；该防护通过 `pager.html_filter` 默认启用。

### 2. 用户体验：方向键上下左右浏览邮箱全部内容

线程视图（thread view）导航全面强化：**上/下方向键**在邮件线程列表中移动时，右侧邮件正文实时联动切换；**左/右方向键**在线程列表与正文窗格之间切换/放大焦点，形成完整的方向键导航链——仅用方向键即可浏览邮箱全部内容。

### 3. 用户体验：缓存优先，启动近乎秒开

IMAP 启动全面改为缓存优先（cache-first）：邮件列表与正文先从本地 sqlite 缓存即时渲染（stale-while-revalidate，网络增量后台静默同步）；邮箱文件夹列表持久化缓存，启动不再等待联网检查；配合 STATUS 计数短路（RFC 4549）与 MSN 索引持久化，消除启动期全量扫描命令。真实账号（QQ 邮箱 INBOX ~5000 封）实测：热启动等待从分钟级降至秒级（中位约 2.7 秒，最快约 1 秒）。

**目录**：

- [亮点介绍](#亮点介绍)
- [安装](#安装)
- [构建](#构建)
  - [Cargo 编译期特性](#cargo-编译期特性)
- [快速开始](#快速开始)
  - [支持的邮件后端](#支持的邮件后端)
  - [邮件发送后端](#邮件发送后端)
  - [功能特性一览](#功能特性一览)
  - [HTML 渲染](#html-渲染)
- [文档](#文档)

## 安装

- 通过 cargo 从源码安装

  从 git 仓库安装：
  ```sh
  cargo install --git https://github.com/kylelee/hardened-meli meli meli_sanitize_html
  ```

### 运行时依赖

安全浏览 HTML 邮件内容需要安装 [w3m](https://github.com/tats/w3m)：HTML 邮件默认先经内置的
`meli_sanitize_html` 清洗，再交给 `w3m` 渲染。请用系统包管理器安装，例如 Debian/Ubuntu 上
`sudo apt install w3m`，Fedora 上 `sudo dnf install w3m`。缺少 w3m 时查看 HTML 邮件会提示
错误并回退显示原始内容。

## 构建

运行 `make` 或 `cargo build --release`。

运行 `make help` 查看 `Makefile` 的使用说明。

详细构建步骤见 [`BUILD.md`](./BUILD.md)。

### Cargo 编译期特性

`meli` 支持通过 cargo features 在编译期选择启用/禁用功能。

`default` feature 的内容为：

```toml
default = ["sqlite3", "notmuch", "smtp", "http", "dbus-notifications", "gpgme", "cli-docs", "jmap", "static"]
```

全部 feature 及其说明如下：

| Feature 标志                                                 | 依赖                                                                                         | 说明                                                                                                                                                                                              |
|--------------------------------------------------------------|----------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| <a name="http-feature">`http`</a>                            | `melib` 的 `http` feature                                                                    | 提供 HTTP 客户端（经 `melib`，供 JMAP 后端使用）                                                                                                                                                  |
| <a name="notmuch-feature">`notmuch`</a>                      | `maildir` feature                                                                            | 提供 *notmuch* 后端                                                                                                                                                                               |
| <a name="jmap-feature">`jmap`</a>                            | `http` feature、启用 `serde` feature 的 `url` crate                                          | 提供 *JMAP* 后端                                                                                                                                                                                  |
| <a name="smtp-feature">`smtp`</a>                            | `tls` feature                                                                                | 内置异步 *SMTP* 客户端                                                                                                                                                                            |
| <a name="sqlite3-feature">`sqlite3`</a>                      | 启用 `bundled-full` feature 的 `rusqlite` crate                                              | 用于缓存                                                                                                                                                                                          |
| <a name="sqlite3-static-feature">`sqlite3-static`</a>        | 启用 `bundled-full` feature 的 `rusqlite` crate                                              | 与 `sqlite3` feature 相同；为一致性保留，以防将来 `sqlite3` feature 停止静态捆绑 libsqlite3。                                                                                                     |
| <a name="smtp-trace-feature">`smtp-trace`</a>                | `smtp` feature                                                                               | 在 `trace` 日志级别输出连接跟踪日志                                                                                                                                                              |
| <a name="gpgme-feature">`gpgme`</a>                          |                                                                                              | 通过动态加载 `libgpgme.so` 使用 *GPG*                                                                                                                                                            |
| <a name="tls-static-feature">`tls-static`</a>                | 启用 `vendored` feature 的 `native-tls` crate                                                | 在使用处静态链接 `OpenSSL`                                                                                                                                                                        |
| <a name="http-static-feature">`http-static`</a>              | 启用 `static-curl` feature 的 `isahc` crate                                                  | 静态链接 `curl`                                                                                                                                                                                   |
| <a name="dbus-notifications-feature">`dbus-notifications`</a> | `notify-rust` 依赖                                                                           | 使用 DBus 通知                                                                                                                                                                                    |
| <a name="dbus-static-feature">`dbus-static`</a>               | `notify-rust` 依赖，并启用其 `d_vendored` feature                                            | 静态捆绑 dbus 库。                                                                                                                                                                                |
| <a name="cli-docs-feature">`cli-docs`</a>                    | `flate2` 依赖                                                                                | 将由 `mandoc` 或 `man` 二进制编译的手册页以纯文本形式内嵌到 `meli` 命令行；内嵌文档可通过子命令 `meli man [PAGE]` 查看                                                                           |
| <a name="libz-static-feature">`libz-static`</a>               | `libz-sys` 依赖，并启用其 `static` feature                                                   | 允许传递依赖 libz（来自 `curl`）被静态链接。                                                                                                                                                      |
| <a name="static-feature">`static`</a>                        | 启用 `tls-static`、`http-static`、`sqlite3-static`、`dbus-static`、`libz-static` features    |                                                                                                                                                                                                   |

## 快速开始

```sh
# 在 ${XDG_CONFIG_HOME}/meli/config.toml 创建配置文件：
$ meli create-config
# 在 ${EDITOR} 或 ${VISUAL} 中编辑配置：
$ meli edit-config
# 可选：通过 cargo 安装时手动安装手册页：
$ meli install-man
# 准备就绪。
$ meli
# 你可以用 CLI 子命令 `man` 阅读任意手册页：
$ meli man meli.7
# 查看帮助输出，了解全部选项与子命令。
$ meli --help
```

在手册页 [`meli(7)`](./meli/docs/meli.7) 中查看 meli 的全面使用教程。

另见在线[快速入门教程](https://meli-email.org/documentation.html#quick-start)。

安装 `meli` 后，可参阅 `meli(1)`、`meli.conf(5)`、`meli(7)` 与 `meli-themes(5)` 获取文档。
示例配置与主题文件位于 `meli/docs/samples/` 子目录。
配置项示例见 `meli.conf.examples(5)`。
手册页也[在线托管](https://meli-email.org/documentation.html "meli documentation")。
`meli` 默认在以下位置查找配置文件：`${XDG_CONFIG_HOME}/meli/config.toml`。

你可以通过设置 `${MELI_CONFIG}` 环境变量指向任意配置文件来运行 meli，例如：

```sh
MELI_CONFIG=./test_config cargo run
```

详见 [`meli(7)`](./meli/docs/meli.7)（完整教程）与 [`meli.conf(5)`](./meli/docs/meli.conf.5)（全部配置项）。

| 主视图 | 紧凑主视图 | 内嵌终端编辑器撰写 |
|--------|-----------|--------------------|
| ![主视图截图](./meli/docs/screenshots/main.webp "mail meli view screenshot") | ![紧凑主视图截图](./meli/docs/screenshots/compact.webp "compact main view screenshot") | ![撰写视图截图](./meli/docs/screenshots/compose.webp "composing view screenshot") |

### 支持的邮件后端

| 协议          | 支持程度   |
|---------------|-----------|
| IMAP          | 完整      |
| Maildir       | 完整      |
| notmuch       | 完整[^0]  |
| mbox          | 只读      |
| JMAP          | 可用      |
| NNTP / Usenet | 可用      |

[^0]: 不支持直接对全部邮件进行搜索；你需要创建一个返回所有邮件的 notmuch 查询邮箱，然后在该邮箱内搜索。

### 邮件发送后端

- SMTP
- 通过管道交给 shell 脚本
- 服务器支持时的服务器端提交

### 功能特性一览

- TLS
- 邮件线程（threading）支持
- 多线程、异步操作
- 可选在 meli 内嵌的 xterm 兼容终端模拟器中运行你喜欢的编辑器
- TOML 纯文本配置
- 可在 UI 标签页中打开邮件并切换
- 可选 sqlite3 索引搜索
- 几乎所有设置都可按邮箱、按账户覆盖
- 联系人列表（+只读 vCard 与 mutt alias 文件支持）
- 强制 UTF-8（其他编码只读）
- 可配置快捷键
- 主题定制（theming）
- `NO_COLOR` 支持
- 纯 ASCII 绘制字符选项
- 通过 html 过滤命令查看 text/html 附件（默认 w3m）
- 附件/邮件可通过管道交给外部程序
- 使用外部附件文件选择器，无需手动输入附件完整路径
- 一条命令或一个快捷键（默认 `C-s`）即可把当前邮件的全部附件保存到 `~/Downloads/meli-<subject>`
- GPG 签名、加密、签名 + 加密
- GPG 签名验证

### HTML 渲染

HTML 邮件默认先经内置的 `meli_sanitize_html` 清洗器（白名单式，移除脚本和危险链接），再交给 [w3m](https://github.com/tats/w3m) 渲染。
可通过 `pager.html_filter` 设置覆盖或关闭（设为 `''` 即不经清洗直接用 w3m 渲染）；详见 [`meli.conf(5)`](./meli/docs/meli.conf.5)。


## 文档

在手册页 [`meli(7)`](./meli/docs/meli.7) 中查看 meli 的全面使用教程。

另见在线[快速入门教程](https://meli-email.org/documentation.html#quick-start)。

安装 `meli` 后，可参阅 `meli(1)`、`meli.conf(5)`、`meli(7)` 与 `meli-themes(5)` 获取文档。
示例配置与主题文件位于 `meli/docs/samples/` 子目录。
手册页也[在线托管](https://meli-email.org/documentation.html "meli documentation")。

`meli` 默认在以下位置查找配置文件：`${XDG_CONFIG_HOME}/meli/config.toml`。

你可以通过设置 `${MELI_CONFIG}` 环境变量指向任意配置文件，或使用 `[-c, --config]` 参数来运行 meli：

```sh
MELI_CONFIG=./test_config meli
```

或

```sh
meli -c ./test_config
```
