// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
use std::collections::{HashMap, HashSet};

/// Tag/attribute/scheme 白名单与 ~/.config/meli/sanitize_html.py 的 nh3.clean()
/// 参数逐项对齐；其余 Builder 设置**刻意保留 ammonia 默认**以维持 nh3 行为
/// 一致（nh3 即 ammonia 的 Python 绑定，锁定同版本 4.1.4）：
/// - link_rel 默认 Some("noopener noreferrer")：nh3 默认相同
/// - url_relative 默认 PassThrough：相对 URL 原样保留，nh3 默认相同
/// - generic_attributes 默认 {"lang","title"}：**勿清空**，nh3 未覆盖时保留同款默认
/// - clean_content_tags 默认 {"script","style"}：连内容一起删除，nh3 相同
pub fn sanitize(input: &str) -> String {
    let tags: HashSet<&str> = [
        "a",
        "b",
        "blockquote",
        "br",
        "code",
        "em",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "li",
        "ol",
        "p",
        "pre",
        "strong",
        "table",
        "td",
        "th",
        "tr",
        "ul",
    ]
    .into();
    let mut tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attributes.insert("a", ["href", "title"].into());
    ammonia::Builder::default()
        .tags(tags)
        .tag_attributes(tag_attributes)
        .url_schemes(["http", "https", "mailto"].into())
        .strip_comments(true)
        .clean(input)
        .to_string()
}
