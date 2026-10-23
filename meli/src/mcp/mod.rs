/*
 * meli - MCP (Model Context Protocol) server for local AI agents.
 *
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
 * along with meli.  If not, see <http://www.gnu.org/licenses/>.
 */

//! `meli mcp`: a stdio MCP server for local AI agents.
//!
//! Read, search, organize and review e-mail — and score the URLs inside
//! it for phishing risk — without ever leaking credentials or message
//! contents to the network.
//!
//! Security model (see `meli/docs/mcp.md` for the full text):
//!
//! - **Read-only by default**: mutating tools refuse to run until the user
//!   adds the matching class to `[mcp] allowed_mutations` in their own
//!   configuration.
//! - **No credential surface**: accounts live in the user's configuration;
//!   MCP has no account/password tools, and responses never include
//!   secrets.
//! - **Untrusted data**: every tool that returns message contents says so
//!   in its description; agents must not follow instructions found inside
//!   e-mail (indirect prompt injection).
//! - **No deletion**: there is deliberately no delete tool.
//! - **Local-only URL scoring**: the engine never fetches a URL and never
//!   sends message contents anywhere.

pub mod protocol;
pub mod scoring;
pub mod session;
pub mod tools;
pub mod urls;

use melib::Result;

/// Entry point of `meli mcp`: serve MCP on stdin/stdout until EOF.
pub fn run() -> Result<()> {
    let mut session = session::Session::new()?;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    protocol::serve(stdin.lock(), &mut out, &mut session)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::{Value, json};

    use super::session::Session;
    use super::*;

    fn line_json(v: &str) -> Value {
        let mut parsed = serde_json::Deserializer::from_str(v).into_iter::<Value>();
        match parsed.next() {
            Some(Ok(v)) => v,
            _ => Value::Null,
        }
    }

    fn serve_input(input: &str, session: &mut Session) -> String {
        let mut out = Vec::new();
        protocol::serve(Cursor::new(input.as_bytes().to_vec()), &mut out, session).unwrap();
        String::from_utf8(out).expect("utf8 output")
    }

    fn default_session() -> Session {
        Session::from_settings(crate::conf::Settings::default()).expect("default settings ok")
    }

    #[test]
    fn test_handshake_tools_list_and_ping() {
        let mut session = default_session();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-03-26\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n",
        );
        let out = serve_input(input, &mut session);
        let mut lines = out.lines();
        let init = line_json(lines.next().expect("init response"));
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "meli");
        let tools_list = line_json(lines.next().expect("tools/list response"));
        let names: Vec<&str> = tools_list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"scan_email_urls"), "{names:?}");
        assert!(names.contains(&"send_email"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("delete")), "{names:?}");
        let ping = line_json(lines.next().expect("ping response"));
        assert_eq!(ping["result"], json!({}));
        assert!(lines.next().is_none(), "notifications get no response");
    }

    #[test]
    fn test_requests_before_initialize_are_rejected() {
        let mut session = default_session();
        let out = serve_input(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
            &mut session,
        );
        let resp = line_json(out.lines().next().expect("response"));
        assert_eq!(resp["error"]["code"], json!(-32002));
    }

    #[test]
    fn test_unknown_method_and_parse_error() {
        let mut session = default_session();
        let out = serve_input(
            concat!(
                "not json at all\n",
                "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"initialize\",\"params\":{}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"bogus/method\"}\n",
            ),
            &mut session,
        );
        let mut lines = out.lines();
        let parse_err = line_json(lines.next().expect("parse error response"));
        assert_eq!(parse_err["error"]["code"], json!(-32700));
        assert_eq!(parse_err["id"], Value::Null);
        let _init = lines.next().expect("init ok");
        let not_found = line_json(lines.next().expect("method not found"));
        assert_eq!(not_found["error"]["code"], json!(-32601));
    }

    #[test]
    fn test_tool_call_success_and_tool_error_shapes() {
        let mut session = default_session();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"score_urls\",\"arguments\":{\"urls\":[\"http://paypal.com.secure-login.example.ru/\"]}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"no_such_tool\",\"arguments\":{}}}\n",
        );
        let out = serve_input(input, &mut session);
        let mut lines = out.lines();
        let _ = lines.next().expect("init");
        let scored = line_json(lines.next().expect("score response"));
        assert_eq!(scored["result"]["isError"], json!(false));
        let text = scored["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["results"][0]["risk"], json!("high"));
        assert!(parsed["results"][0]["signals"].as_array().unwrap().len() >= 2);
        let missing = line_json(lines.next().expect("unknown tool response"));
        assert_eq!(missing["result"]["isError"], json!(true));
        assert!(
            missing["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("unknown tool")
        );
    }

    #[test]
    fn test_resources_and_prompts() {
        let mut session = default_session();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"resources/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"resources/read\",\"params\":{\"uri\":\"email://accounts\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"prompts/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"prompts/get\",\"params\":{\"name\":\"triage_inbox\",\"arguments\":{\"account\":\"work\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"prompts/get\",\"params\":{\"name\":\"triage_inbox\",\"arguments\":{}}}\n",
        );
        let out = serve_input(input, &mut session);
        let mut lines = out.lines();
        let _ = lines.next().expect("init");
        let res_list = line_json(lines.next().expect("resources/list"));
        assert_eq!(
            res_list["result"]["resources"][0]["uri"],
            "email://accounts"
        );
        let res_read = line_json(lines.next().expect("resources/read"));
        assert!(
            res_read["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("accounts")
        );
        let prompt_list = line_json(lines.next().expect("prompts/list"));
        assert_eq!(
            prompt_list["result"]["prompts"].as_array().unwrap().len(),
            2
        );
        let prompt = line_json(lines.next().expect("prompts/get"));
        let text = prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("`work`"));
        let bad = line_json(lines.next().expect("prompts/get missing arg"));
        assert_eq!(bad["error"]["code"], json!(-32602));
    }

    #[test]
    fn test_default_read_only_rejects_mutations() {
        let mut session = default_session();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"set_flags\",\"arguments\":{\"account\":\"any\",\"mailbox\":\"INBOX\",\"email_ids\":[\"1\"],\"flags\":{\"seen\":true}}}}\n",
        );
        let out = serve_input(input, &mut session);
        let mut lines = out.lines();
        let _ = lines.next().expect("init");
        let resp = line_json(lines.next().expect("set_flags response"));
        assert_eq!(resp["result"]["isError"], json!(true));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("allowed_mutations"), "{text}");
        assert!(text.contains("organize"), "{text}");
    }
}
