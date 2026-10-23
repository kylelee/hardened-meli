<!-- SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later -->

---
name: url-trust
description: 评估邮件内容中 URL 的可信度，输出 (0,1) 开区间内的置信度评分。输入一个 URL（可选链接显示文本、发件人等上下文），按词法规则逐条打分并返回严格 JSON 结果。
version: 1.0
---

# URL 可信度评分 Skill（url-trust）

## 1. 目标与适用范围

本 skill 用于评估**从邮件内容中提取的 URL** 是否可信，供 AI agent
（人工智能代理）调用。

- 评估对象：单个 URL 字符串，必要时附带邮件上下文。
- 方法：**纯静态词法分析**（static lexical analysis）——只依据 URL
  自身的结构特征判定，默认不联网、不访问目标站点。
- 输出：一个 **(0, 1) 开区间**内的置信度（confidence）。注意是开区间：
  任何情况下都**不得输出恰好 0.00 或 1.00**——静态分析永远存在不确定
  性，绝对肯定与绝对否定都不诚实。

## 2. 调用方式

用户对 AI agent 说（或任何等价表达）：

> 用 url-trust skill 评估这个 URL：`<URL>`
> 可选上下文：显示文本 `<...>`、发件人 `<...>`、链接周围文字 `<...>`

agent 读取本文件后，严格按第 5 节流程执行，并按第 6 节格式输出。

## 3. 输入

| 字段          | 必填 | 说明                                         |
|---------------|------|----------------------------------------------|
| url           | 是   | 待评估的 URL 原文                            |
| display_text  | 否   | 邮件里该链接的显示文本（锚文本）             |
| sender        | 否   | 邮件发件人地址                               |
| context       | 否   | 链接周围的一句话上下文                       |

## 4. 评分规则

基准分 `BASE = 0.50`。逐条匹配下表信号并累加权重；命中"封顶"
（cap）规则的，最终分不得超过其 cap；最后收敛进 [0.01, 0.99]。

### 4.1 加分信号

| ID | 信号                                                   | 权重  |
|----|--------------------------------------------------------|-------|
| P1 | 注册域（registrable domain，eTLD+1）属于已知高信誉域   | +0.30 |
| P2 | scheme 为 https                                        | +0.05 |
| P3 | 有上下文且链接注册域与发件人注册域一致                 | +0.10 |
| P4 | 路径/查询干净：无重定向参数、无凭据诱导词、无嵌套 URL  | +0.05 |

P1 种子白名单（agent 可按自身知识扩展）：gnu.org、github.com、
gitlab.com、google.com、youtube.com、wikipedia.org、mozilla.org、
apache.org、debian.org、kernel.org、rust-lang.org、crates.io、
microsoft.com、apple.com、amazon.com、cloudflare.com、
stackoverflow.com、qq.com、163.com、taobao.com、alipay.com、
meli-email.org 等。两点约束：

- 只匹配**注册域**（eTLD+1 整体），不是子串匹配——
  `github.com.evil.io` 的注册域是 evil.io，不加分；
- URL 缩短服务域（bit.ly、t.co 等）**不适用 P1**：知名度不等于
  可信，其最终目标不透明（见 S1）。

### 4.2 致命信号（命中即封顶）

| ID | 信号                                                                                          | cap  |
|----|-----------------------------------------------------------------------------------------------|------|
| F1 | 危险 scheme：`javascript:`、`data:`、`vbscript:`、`file:`、`blob:`                            | 0.05 |
| F2 | 主机为裸 IP 字面量（十进制、十六进制、八进制或整数编码，如 `0x7f000001`）                      | 0.10 |
| F3 | punycode 标签（`xn--`）且解码后与知名品牌同形（homograph，如 `xn--80ak6aa92e.com` → аpple.com）| 0.05 |
| F4 | 与知名品牌注册域编辑距离 ≤ 1 的 typosquatting（如 goog1e.com、paypa1.com、rnicrosoft.com）     | 0.10 |
| F5 | 知名品牌词只出现在子域或路径，注册域却是别的域（如 `paypal.com.secure-verify.io`）             | 0.10 |
| F6 | 可执行/危险文件扩展名：双后缀（invoice.pdf.exe）或 .exe/.scr/.msi/.bat/.cmd/.js/.vbs/.hta/.apk/.jar/.iso | 0.10 |
| F7 | 有上下文时：显示文本中的域名 ≠ href 实际注册域（邮件钓鱼经典手法）                             | 0.15 |

品牌词匹配池（F3/F4/F5 共用，agent 可扩展）：paypal、apple、google、
microsoft、amazon、alibaba、taobao、qq、163、weibo、icloud、facebook、
netflix、instagram、linkedin、github、dropbox、dhl、fedex、chase、
hsbc、coinbase、binance 等。

### 4.3 强扣分信号

| ID | 信号                                                                                   | 权重          |
|----|----------------------------------------------------------------------------------------|---------------|
| S1 | URL 缩短服务（bit.ly、t.co、tinyurl.com、is.gd、ow.ly、cutt.ly、shorturl.at、rb.gy、s.id 等） | −0.30，另封顶 0.60 |
| S2 | 开放重定向参数（url=、redirect=、next=、u=、target=、continue=、dest= 等）指向另一主机  | −0.40         |
| S3 | URL 内嵌套另一个完整 URL（双 scheme）                                                  | −0.35         |
| S4 | 凭据诱导词（login、verify、secure、account、update、wallet、signin、confirm）与品牌词同现于路径 | −0.30         |
| S5 | userinfo 欺骗：主机前带 `user[:pass]@`                                                 | −0.50         |
| S6 | 高滥用 TLD：.zip、.mov、.top、.xyz、.tk、.ml、.ga、.cf、.gq、.icu、.cyou、.cam、.rest、.sbs | −0.20         |
| S7 | 主机标签随机性高：高熵字符串、数字字母混杂、连字符 ≥ 3                                 | −0.15         |
| S8 | 子域深度 ≥ 4 层                                                                         | −0.10         |
| S9 | 非标准端口（非 80/443）                                                                 | −0.15         |
| S10| 明文 http 且路径含登录/账户类词                                                         | −0.25         |
| S11| 主机大量百分号编码 / 大小写混淆（bAdCoM）                                               | −0.15         |
| S12| 有上下文时：发件人注册域与链接注册域不同，且上下文/显示文本声称是某品牌                 | −0.25         |

### 4.4 中性 scheme

`mailto:`、`tel:`、`news:` 等**非网页导航** scheme 不适用 F1：以 0.60
为基准分（邮件场景中性偏正），只受上下文规则调整。

## 5. 评分流程（伪代码）

```
1. 解析 URL → scheme / userinfo / host / port / path / query / fragment
   解析失败 → 按"失败处理"输出 error，不给分。
2. 规范化：解码 xn-- 标签比对；主机与路径做 percent-decode；Unicode NFKC。
3. 逐条匹配 4.1–4.3 规则，记录每个命中信号 {id, rule, weight, evidence}。
4. score = BASE + Σ(命中权重)
5. 对每个命中的封顶规则：score = min(score, cap)
6. score = clamp(score, 0.01, 0.99)，保留两位小数
7. 分档：
   score ≥ 0.75        → verdict = "likely-trusted"（倾向可信）
   0.40 ≤ score < 0.75 → verdict = "uncertain"（存疑）
   score < 0.40        → verdict = "likely-malicious"（倾向不可信）
```

## 6. 输出格式

严格 JSON，并在 JSON 之后附一行中文摘要：

```json
{
  "url": "<原始输入>",
  "confidence": 0.87,
  "verdict": "likely-trusted",
  "signals": [
    {"id": "P1", "rule": "注册域属于已知高信誉域", "weight": 0.30,
     "evidence": "注册域 github.com 命中种子白名单"},
    {"id": "P2", "rule": "scheme 为 https", "weight": 0.05,
     "evidence": "scheme=https"}
  ],
  "notes": "一句话普通人能看懂的结论"
}
```

硬性约束：

- `confidence` 必须是 (0, 1) 开区间内的数，保留两位小数，
  **永不输出 0.00 或 1.00**；
- `signals` 按命中顺序列出**全部**证据（含加分与扣分），不得只报坏消息；
- `notes` 用一句话点明最关键的判定原因。

## 7. 校准样例（agent 自检基准）

| 输入（及上下文）                                        | 期望 confidence | 命中          |
|---------------------------------------------------------|-----------------|---------------|
| https://www.gnu.org/software/meli/                      | 0.85–0.97       | P1+P2+P4      |
| https://github.com/kylelee/hardened-meli                | 0.85–0.97       | P1+P2+P4      |
| https://accounts.google.com/ServiceLogin                | 0.80–0.95       | P1+P2（知名域自身业务，见 §8） |
| mailto:team@example.com                                 | 0.55–0.70       | 中性 scheme   |
| http://example.com/                                     | 0.40–0.55       | 良构但明文+未知域 |
| https://bit.ly/3xYz9                                    | 0.20–0.40       | S1（不透明封顶 0.60 后扣减） |
| https://good.com/redirect?url=http://evil.io            | 0.10–0.25       | S2（与 S3 同一嵌套不双扣） |
| 显示文本 "https://apple.com"，实际 https://apple-id-verify.xyz/login | ≤0.15 | F7+S6+S4 |
| https://paypal.com.secure-verify.io/login               | 0.03–0.10       | F5 封顶       |
| https://rnicrosoft.com/login                            | 0.03–0.10       | F4 封顶（rn→m 视觉混淆） |
| https://xn--80ak6aa92e.com/                             | 0.02–0.05       | F3 封顶（аpple 同形攻击） |
| http://192.168.1.4/admin                                | 0.03–0.10       | F2 封顶       |
| https://example.com/download/invoice.pdf.exe            | 0.03–0.10       | F6 封顶       |
| javascript:alert(1)                                     | 0.01–0.05       | F1 封顶       |

## 8. 失败处理与边界

- 输入无法解析为 URL → 输出 `{"error": "...", "confidence": null}`，
  不得硬给分；
- localhost、单标签主机、点域 → 按 F2 处理（等价裸主机）；
- 大小写、末尾斜杠等无关差异不影响评分；
- **同一证据不得被两条规则重复扣分**：S2 与 S3 指向同一嵌套 URL 时，
  只计绝对值更大的一条（S2）；F3 与 F4 同时命中时权重可叠加，但取
  更严的 cap，cap 之间不叠加（取 min）；
- **判定对象是冒充者，不是品牌本尊**：P1 命中的知名域，其自身路径里
  的登录/账户词（如 accounts.google.com/ServiceLogin）不触发 S4/F5。

## 9. 联网增强（可选，默认关闭）

默认**纯本地词法分析，不上传 URL 到任何外部服务**。仅当用户明确要求
联网增强时，才可对**注册域**（而非完整 URL）做信誉检索（WHOIS 域龄、
威胁情报标记等），调整幅度限制在 ±0.10 之内，且必须在对应 evidence
中注明来源。

## 10. 局限性声明

- 静态词法分析识别不了：被入侵的合法网站、新注册尚未被标记的钓鱼域、
  纯靠话术的社会工程；
- confidence 是**置信度**，不是证明：< 0.40 不等于"确认恶意"，
  ≥ 0.75 不等于"绝对安全"；
- 结论必须附带 signals 证据链，让用户可以自行复核每一条判定依据。

## 11. 维护

规则或权重有任何改动，必须同步更新 §7 校准样例，使两者保持一致，
并递增 frontmatter 中的 version。
