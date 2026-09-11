//
// melib
//
// Copyright 2017 Emmanouil Pitsidianakis <manos@pitsidianak.is>
//
// This file is part of meli.
//
// meli is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// meli is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with meli. If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

use crate::email::{
    address::*,
    parser::{
        address::*,
        attachments,
        dates::rfc5322_date,
        encodings::*,
        generic::{comment, phrase2, unstructured},
        headers,
        mailing_lists::rfc_2369_list_headers_action_list,
        BytesExt,
    },
};

macro_rules! to_str {
    ($l:expr) => {{
        unsafe { std::str::from_utf8_unchecked($l) }
    }};
}

pub struct MimeEncodedWord {
    pub encoded: &'static [u8],
    pub decoded: &'static str,
}

pub const MIME_ENCODED_WORDS: &[MimeEncodedWord] = &[
    // No encoding
    MimeEncodedWord {
        encoded: b"sdf",
        decoded: "sdf"
    },
    // UTF-8
    MimeEncodedWord {
        encoded: br#"=?UTF-8?Q?foo=20=22=20bar?="#,
        decoded: "foo \" bar"
    },
    MimeEncodedWord {
        encoded: b"=?UTF-8?Q?=CE=A0=CF=81=CF=8C=CF=83=CE=B8=CE=B5?= =?UTF-8?Q?=CF=84=CE=B7_=CE=B5=CE=BE=CE=B5=CF=84?= =?UTF-8?Q?=CE=B1=CF=83=CF=84=CE=B9=CE=BA=CE=AE?=",
        decoded: "Πρόσθετη εξεταστική"
    },
    MimeEncodedWord {
        encoded: b"[Advcomparch] =?utf-8?b?zqPPhc68z4DOtc+BzrnPhs6/z4HOrCDPg861IGZs?=\n\t=?utf-8?b?dXNoIM67z4zOs8+JIG1pc3ByZWRpY3Rpb24gzrrOsc+Ezqwgz4TOt869?=\n\t=?utf-8?b?IM61zrrPhM6tzrvOtc+Dzrcgc3RvcmU=?=",
        decoded: "[Advcomparch] Συμπεριφορά σε flush λόγω misprediction κατά την εκτέλεση store"
    },
    MimeEncodedWord {
        encoded: b"Re: [Advcomparch] =?utf-8?b?zqPPhc68z4DOtc+BzrnPhs6/z4HOrCDPg861IGZs?=
=?utf-8?b?dXNoIM67z4zOs8+JIG1pc3ByZWRpY3Rpb24gzrrOsc+Ezqwgz4TOt869?=
=?utf-8?b?IM61zrrPhM6tzrvOtc+Dzrcgc3RvcmU=?=",
        decoded: "Re: [Advcomparch] Συμπεριφορά σε flush λόγω misprediction κατά την εκτέλεση store"
    },
    MimeEncodedWord {
        encoded: br#"[internal] =?UTF-8?B?zp3Orc6/z4Igzp/OtM63zrPPjM+CIM6jz4XOs86zz4E=?=
                =?UTF-8?B?zrHPhs6uz4I=?="#,
        decoded: "[internal] Νέος Οδηγός Συγγραφής"
    },
    MimeEncodedWord {
        encoded: br#"=?UTF-8?Q?Re=3a_Climate_crisis_reality_check_=e2=80=93=c2=a0EcoHust?=
                =?UTF-8?Q?ler?="#,
        decoded: "Re: Climate crisis reality check –\u{a0}EcoHustler"
    },
    MimeEncodedWord {
        encoded: br#"=?UTF-8?Q?Invitation=3A_Test_event_=E2=8F=B0_=40_Thu_Apr_11=2C_2024_11am_=2D?=
                    =?UTF-8?Q?_12pm_=28GMT=2B3=29_=28user=40example=2Ecom=29?="#,
        decoded: "Invitation: Test event ⏰ @ Thu Apr 11, 2024 11am - 12pm (GMT+3) (user@example.com)"
    },
    MimeEncodedWord {
        encoded: br#"=?UTF-8?Q?thisisatest123_"test"?="#,
        decoded: "thisisatest123 \"test\""
    },
    MimeEncodedWord {
        encoded: b"=?utf-8?Q?{#D=C3=A8=C3=A9=C2=A3=C3=A5=C3=BD_M$=C3=A1=C3?= =?utf-8?Q?=AD.=C3=A7=C3=B8m}?= <user@domain.com>",
        decoded: "{#Dèé£åý M$áí.çøm} <user@domain.com>"
    },
    // windows-1250
    MimeEncodedWord {
        encoded: br#"Re: Climate crisis reality check =?windows-1250?B?lqBFY29IdXN0?=
                =?windows-1250?B?bGVy?="#,
        decoded: "Re: Climate crisis reality check –\u{a0}EcoHustler"
    },
    // windows-1251
    MimeEncodedWord {
        encoded: br#"=?WINDOWS-1251?Q?=C0=F0=F2=E5=EC?="#,
        decoded: "Артем"
    },
    // gb18030
    MimeEncodedWord {
        encoded: br#"=?gb18030?B?zNrRtsbz0rXTys/k19S2r9eqt6LR6dak08q8/g==?="#,
        decoded: "腾讯企业邮箱自动转发验证邮件"
    },
    // GBK
    MimeEncodedWord {
        encoded: br#"=?GBK?B?1cK5scf4s8e53L7WudjT2s34wufT38fp0MXPoteo?=
                   =?GBK?B?sai1xLTwuLQoz8K6vszBMbrFKS5kb2M=?="#,
        decoded: "章贡区城管局关于网络舆情信息专报的答复(下壕塘1号).doc"
    },
    // KOI-8R
    MimeEncodedWord {
        encoded: br#"=?KOI8-R?B?4sHCyd7F1yDzxdLHxco=?="#,
        decoded: "Бабичев Сергей"
    },
    // ISO-8859-1
    MimeEncodedWord {
        encoded: br#"=?iso-8859-1?Q?=A1Hola,_se=F1or!?="#,
        decoded: "¡Hola, señor!"
    },
    MimeEncodedWord {
        encoded: br#"=?iso-8859-1?q?=C5ngstr=F6m_=E5ngstr=F6m?="#,
        decoded: "Ångström ångström"
    },
    MimeEncodedWord {
        encoded: br#"=?iso-8859-1?q?Fran=E7ois?="#,
        decoded: "François"
    },
    MimeEncodedWord {
        encoded: br#"=?iso-8859-1?b?VIH1aXZvIExlZWRqgeRydg==?="#,
        decoded: "T\u{81}õivo Leedj\u{81}ärv"
    },
    MimeEncodedWord {
        encoded: b"=?ISO-8859-1?B?SWYgeW91IGNhbiByZWFkIHRoaXMgeW8=?==?ISO-8859-2?B?dSB1bmRlcnN0YW5kIHRoZSBleGFtcGxlLg==?==?US-ASCII?Q?.._cool!?=",
        decoded: "If you can read this you understand the example... cool!",
    },
    // ISO-8859-2
    MimeEncodedWord {
        encoded: br#"=?ISO-8859-2?Q?TEST?="#,
        decoded: "TEST"
    },
    // ISO-8859-7
    MimeEncodedWord {
        encoded: b"=?iso-8859-7?B?W215Y291cnNlcy5udHVhLmdyIC0gyvXs4fTp6t4g6uHpIMri4e306ere?=
       =?iso-8859-7?B?INb18+nq3l0gzd3hIMHt4erv3+358+c6IMzF0c/TIMHQz9TFy8XTzMHU?=
=?iso-8859-7?B?2c0gwiDUzC4gysHNLiDFzsXUwdPH0yAyMDE3LTE4OiDTx8zFydnTxw==?=",
        decoded: "[mycourses.ntua.gr - Κυματική και Κβαντική Φυσική] Νέα Ανακοίνωση: ΜΕΡΟΣ ΑΠΟΤΕΛΕΣΜΑΤΩΝ Β \
    ΤΜ. ΚΑΝ. ΕΞΕΤΑΣΗΣ 2017-18: ΣΗΜΕΙΩΣΗ"
    },
    MimeEncodedWord {
        encoded: b"=?iso-8859-7?b?U2VnIGZhdWx0IPP05+0g5er03evl8+cg9O/1?= =?iso-8859-7?q?_example_ru_n_=5Fsniper?=",
        decoded: "Seg fault στην εκτέλεση του example ru n _sniper"
    },
    MimeEncodedWord {
        encoded: b"Re: [Advcomparch]
=?iso-8859-7?b?U2VnIGZhdWx0IPP05+0g5er03evl8+cg9O/1?=
=?iso-8859-7?q?_example_ru_n_=5Fsniper?=",
        decoded: "Re: [Advcomparch] Seg fault στην εκτέλεση του example ru n _sniper"
    },
    // ks_c_5601-1987
    MimeEncodedWord {
        encoded: b"=?ks_c_5601-1987?B?MTMwMTE3X8HWwvfA5V+1tcDlX7jetLq+8y5wZGY=?=",
        decoded: "130117_주차장_도장_메뉴얼.pdf"
    },
    // ISO-2022-JP
    MimeEncodedWord {
        encoded: br#"=?ISO-2022-JP?B?GyRCM1g5OzU7PVEwdzgmPSQ4IUYkMnFKczlwGyhC?="#,
        decoded: "学校技術員研修検討会報告"
    },
];

#[test]
fn test_email_parser_phrase() {
    for word in MIME_ENCODED_WORDS {
        assert_eq!(
            word.decoded,
            std::str::from_utf8(&phrase(word.encoded, false).unwrap().1).unwrap()
        );
    }
}

#[test]
fn test_email_parser_address_list() {
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![
                Address::new(Some("Obit Oppidum"), "user@domain"),
                Address::new(Some("list"), "list@domain.tld"),
                Address::new(Some("list2"), "list2@domain.tld"),
                Address::new(Some("Bobit Boppidum"), "user@otherdomain.com"),
                Address::new(Some("Cobit Coppidum"), "user2@otherdomain.com"),
                Address::new(None::<&str>, "user@domain.tld")
            ]
        ),
        rfc2822address_list(b"Obit Oppidum <user@domain>, list <list@domain.tld>, list2 <list2@domain.tld>, Bobit Boppidum <user@otherdomain.com>, Cobit Coppidum <user2@otherdomain.com>, <user@domain.tld>").unwrap()
    );
    assert_eq!(
        crate::Error::from(
            rfc2822address_list(b"Prof. Einstein <einstein@example.org>").unwrap_err()
        )
        .to_string(),
        "Error when parsing: \"Prof. Einstein <einstein@example.org>\"\nAddress `Prof. Einstein \
         <einstein@example.org>` contains errors: Display names that contain ()<>[]:;@\\,.\" must \
         be quoted. Try: `\"Prof. Einstein\" <einstein@example.org>` instead",
    );
    // Examples from RFC5322 Appendix A.1.2. "Different Types of Mailboxes"
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![Address::new(
                Some("Joe Q. Public"),
                "john.q.public@example.com"
            )],
        ),
        rfc2822address_list(br#""Joe Q. Public" <john.q.public@example.com>"#).unwrap()
    );
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![
                Address::new(Some("Mary Smith"), "mary@example.com"),
                Address::new(None::<&str>, "jdoe@example.org"),
                Address::new(Some("Who?"), "one@example.com"),
            ],
        ),
        rfc2822address_list(
            br#"Mary Smith <mary@example.com>, jdoe@example.org, Who? <one@example.com>"#
        )
        .unwrap()
    );
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![
                Address::new(None::<&str>, "boss@example.com"),
                Address::new(Some("Giant; \"Big\" Box"), "sysservices@example.com"),
            ]
        ),
        rfc2822address_list(
            br#"<boss@example.com>, "Giant; \"Big\" Box" <sysservices@example.com>"#
        )
        .unwrap()
    );
    // Examples from RFC5322 Appendix A.1.3. "Group Addresses"
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![Address::new_group(
                "A Group".to_string(),
                vec![
                    Address::new(Some("Ed Jones"), "c@a.test"),
                    Address::new(None::<&str>, "joe@where.test"),
                    Address::new(Some("John"), "jdoe@one.test"),
                ]
            )]
        ),
        rfc2822address_list(br#"A Group:Ed Jones <c@a.test>,joe@where.test,John <jdoe@one.test>;"#)
            .unwrap()
    );
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![Address::new_group(
                "Undisclosed recipients".to_string(),
                vec![]
            )]
        ),
        rfc2822address_list(br#"Undisclosed recipients:;"#).unwrap()
    );
}

#[test]
fn test_email_parser_whitespace_comments_and_other_oddities() {
    // Examples from RFC5322 Appendix A.5. White Space, Comments, and Other Oddities
    assert_eq!(
        rfc5322_date(
            br#"Thu,
      13
        Feb
          1999
      23:32
               -0330 (Newfoundland Time)"#
        )
        .unwrap(),
        rfc5322_date(b"Thu, 13 Feb 1999 23:32 -0330").unwrap()
    );
    assert_eq!(
        rfc5322_date(b"Thu, 13 Feb 1999 23:32 -0330").unwrap(),
        rfc5322_date(b"Thu, 13 Feb 1999 23:32:00 -0330").unwrap()
    );
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![Address::new(Some("Pete"), "pete@silly.test")]
        ),
        rfc2822address_list(br#"Pete(A nice \) chap) <pete(his account)@silly.test(his host)>"#)
            .unwrap()
    );
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![Address::new_group(
                "A Group",
                vec![
                    Address::new(Some("Chris Jones"), "c@public.example"),
                    Address::new(None::<&str>, "joe@example.org"),
                    Address::new(Some("John"), "jdoe@one.test"),
                ]
            )]
        ),
        rfc2822address_list(
            br#"A Group(Some people)
     :Chris Jones <c@(Chris's host.)public.example>,
         joe@example.org,
  John <jdoe@one.test> (my dear friend); (the end of the group)"#
        )
        .unwrap()
    );
    assert_eq!(
        (
            [].as_slice(),
            smallvec::smallvec![Address::new_group("Hidden recipients".to_string(), vec![])]
        ),
        rfc2822address_list(br#"(Empty list)(start)Hidden recipients  :(nobody(that I know))  ;"#)
            .unwrap()
    );
}

#[test]
fn test_email_parser_addresses() {
    macro_rules! assert_parse {
        ($name:literal, $addr:literal, $raw:literal) => {{
            #[allow(clippy::string_lit_as_bytes)]
            let s = $raw.as_bytes();
            let r = address(s).unwrap().1;
            match r {
                Address::Mailbox(ref m) => {
                    assert_eq!(
                        (
                            m.display_name.as_deref().unwrap_or_default(),
                            m.address_spec.as_ref()
                        ),
                        ($name, $addr)
                    );
                }
                _ => assert!(false),
            }
        }};
    }

    assert_parse!(
        "Σταύρος Μαλτέζος",
        "maltezos@central.ntua.gr",
        "=?iso-8859-7?B?0/Th/fHv8iDM4ev03ebv8g==?= <maltezos@central.ntua.gr>"
    );
    assert_parse!("", "user@domain", "user@domain");
    assert_parse!("", "user@domain", "<user@domain>");
    assert_parse!("", "user@domain", "  <user@domain>");
    assert_parse!("Name", "user@domain", "Name <user@domain>");
    assert_parse!(
        "",
        "julia@ficdep.minitrue",
        "julia(outer party)@ficdep.minitrue"
    );
    assert_parse!(
        "Winston Smith",
        "winston.smith@recdep.minitrue",
        "\"Winston Smith\" <winston.smith@recdep.minitrue> (Records Department)"
    );
    assert_parse!(
        "John Q. Public",
        "JQB@bar.com",
        "\"John Q. Public\" <JQB@bar.com>"
    );
    assert_parse!(
        "John Q. Public",
        "JQB@bar.com",
        "John \"Q.\" Public <JQB@bar.com>"
    );
    assert_parse!(
        "John Q. Public",
        "JQB@bar.com",
        "\"John Q.\" Public <JQB@bar.com>"
    );
    assert_parse!(
        "John Q. Public",
        "JQB@bar.com",
        "John \"Q. Public\" <JQB@bar.com>"
    );
    assert_parse!(
        "Jeffrey Stedfast",
        "fejj@helixcode.com",
        "Jeffrey Stedfast <fejj@helixcode.com>"
    );
    assert_parse!(
        "this is\ta folded name",
        "folded@name.com",
        "this is\n\ta folded name <folded@name.com>"
    );
    assert_parse!(
        "Jeffrey fejj Stedfast",
        "fejj@helixcode.com",
        "Jeffrey fejj Stedfast <fejj@helixcode.com>"
    );
    assert_parse!(
        "Jeffrey fejj Stedfast",
        "fejj@helixcode.com",
        "Jeffrey \"fejj\" Stedfast <fejj@helixcode.com>"
    );
    assert_parse!(
        "Jeffrey \"fejj\" Stedfast",
        "fejj@helixcode.com",
        "\"Jeffrey \\\"fejj\\\" Stedfast\" <fejj@helixcode.com>"
    );
    assert_parse!(
        "Stedfast, Jeffrey",
        "fejj@helixcode.com",
        "\"Stedfast, Jeffrey\" <fejj@helixcode.com>"
    );
    assert_parse!(
        "",
        "fejj@helixcode.com",
        "fejj@helixcode.com (Jeffrey Stedfast)"
    );
    assert_parse!(
        "Jeffrey Stedfast",
        "fejj@helixcode.com",
        "Jeffrey Stedfast <fejj(nonrecursive block)@helixcode.(and a comment here)com>"
    );
    assert_parse!(
        "Jeffrey Stedfast",
        "fejj@helixcode.com",
        "Jeffrey Stedfast <fejj(recursive (comment) block)@helixcode.(and a comment here)com>"
    );
    assert_parse!(
        "Joe Q. Public",
        "john.q.public@example.com",
        "\"Joe Q. Public\" <john.q.public@example.com>"
    );
    assert_parse!("Mary Smith", "mary@x.test", "Mary Smith <mary@x.test>");
    assert_parse!("Mary Smith", "mary@x.test", "Mary Smith <mary@x.test>");
    assert_parse!("", "jdoe@example.org", "jdoe@example.org");
    assert_parse!("Who?", "one@y.test", "Who? <one@y.test>");
    assert_parse!("", "boss@nil.test", "<boss@nil.test>");
    assert_parse!(
        "Giant; \"Big\" Box",
        "sysservices@example.net",
        r#""Giant; \"Big\" Box" <sysservices@example.net>"#
    );
    //assert_eq!(
    //    Address::new(Some("Jeffrey Stedfast"), "fejj@helixcode.com"),
    //    address(b"Jeffrey Stedfast <fejj@helixcode.com.>")
    //        .unwrap()
    //        .1
    //);
    assert_parse!(
        "John <middle> Doe",
        "jdoe@machine.example",
        "\"John <middle> Doe\" <jdoe@machine.example>"
    );
    // RFC 2047 "Q"-encoded ISO-8859-1 address.
    assert_parse!(
        "Jörg Doe",
        "joerg@example.com",
        "=?iso-8859-1?q?J=F6rg_Doe?= <joerg@example.com>"
    );

    // RFC 2047 "Q"-encoded US-ASCII address. Dumb but legal.
    assert_parse!(
        "Jorg Doe",
        "joerg@example.com",
        "=?us-ascii?q?J=6Frg_Doe?= <joerg@example.com>"
    );
    // RFC 2047 "Q"-encoded UTF-8 address.
    assert_parse!(
        "Jörg Doe",
        "joerg@example.com",
        "=?utf-8?q?J=C3=B6rg_Doe?= <joerg@example.com>"
    );
    // RFC 2047 "Q"-encoded UTF-8 address with multiple encoded-words.
    assert_parse!(
        "JörgDoe",
        "joerg@example.com",
        "=?utf-8?q?J=C3=B6rg?=  =?utf-8?q?Doe?= <joerg@example.com>"
    );
    assert_parse!(
        "André Pirard",
        "PIRARD@vm1.ulg.ac.be",
        "=?ISO-8859-1?Q?Andr=E9?= Pirard <PIRARD@vm1.ulg.ac.be>"
    );
    // Custom example of RFC 2047 "B"-encoded ISO-8859-1 address.
    assert_parse!(
        "Jörg",
        "joerg@example.com",
        "=?ISO-8859-1?B?SvZyZw==?= <joerg@example.com>"
    );
    // Custom example of RFC 2047 "B"-encoded UTF-8 address.
    assert_parse!(
        "Jörg",
        "joerg@example.com",
        "=?UTF-8?B?SsO2cmc=?= <joerg@example.com>"
    );
    // Custom example with "." in name. For issue 4938
    //assert_parse!(
    //    "Asem H.",
    //    "noreply@example.com",
    //    "Asem H. <noreply@example.com>"
    //);
    assert_parse!(
        // RFC 6532 3.2.3, qtext /= UTF8-non-ascii
        "Gø Pher",
        "gopher@example.com",
        "\"Gø Pher\" <gopher@example.com>"
    );
    // RFC 6532 3.2, atext /= UTF8-non-ascii
    assert_parse!("µ", "micro@example.com", "µ <micro@example.com>");
    // RFC 6532 3.2.2, local address parts allow UTF-8
    //assert_parse!("Micro", "µ@example.com", "Micro <µ@example.com>");
    // RFC 6532 3.2.4, domains parts allow UTF-8
    //assert_parse!(
    //    "Micro",
    //    "micro@µ.example.com",
    //    "Micro <micro@µ.example.com>"
    //);
    // Issue 14866
    assert_parse!(
        "",
        "emptystring@example.com",
        "\"\" <emptystring@example.com>"
    );
    // CFWS
    assert_parse!(
        "",
        "cfws@example.com",
        "<cfws@example.com> (CFWS (cfws))  (another comment)"
    );
    //"<cfws@example.com> ()  (another comment), <cfws2@example.com> (another)"
    assert_parse!(
        "Kristoffer Brånemyr",
        "ztion@swipenet.se",
        "=?iso-8859-1?q?Kristoffer_Br=E5nemyr?= <ztion@swipenet.se>"
    );
    assert_parse!(
        "François Pons",
        "fpons@mandrakesoft.com",
        "=?iso-8859-1?q?Fran=E7ois?= Pons <fpons@mandrakesoft.com>"
    );
    assert_parse!(
        "هل تتكلم اللغة الإنجليزية /العربية؟",
        "do.you.speak@arabic.com",
        "=?utf-8?b?2YfZhCDYqtiq2YPZhNmFINin2YTZhNi62Kkg2KfZhNil2YbYrNmE2YrYstmK2Kk=?=\n \
         =?utf-8?b?IC/Yp9mE2LnYsdio2YrYqdif?= <do.you.speak@arabic.com>"
    );
    assert_parse!(
        "狂ったこの世で狂うなら気は確かだ。",
        "famous@quotes.ja",
        "=?utf-8?b?54uC44Gj44Gf44GT44Gu5LiW44Gn54uC44GG44Gq44KJ5rCX44Gv56K644GL44Gg?=\n \
         =?utf-8?b?44CC?= <famous@quotes.ja>"
    );
    assert_eq!(
        Address::new_group(
            "A Group".to_string(),
            vec![
                Address::new(Some("Ed Jones"), "c@a.test"),
                Address::new(None::<&str>, "joe@where.test"),
                Address::new(Some("John"), "jdoe@one.test")
            ]
        ),
        address(b"A Group:Ed Jones <c@a.test>,joe@where.test,John <jdoe@one.test>;")
            .unwrap()
            .1
    );
    assert_eq!(
        Address::new_group("Undisclosed recipients".to_string(), vec![]),
        address(b"Undisclosed recipients:;").unwrap().1
    );
    assert_parse!(
        "狂ったこの世で狂うなら気は確かだ。",
        "famous@quotes.ja",
        "狂ったこの世で狂うなら気は確かだ。 <famous@quotes.ja>"
    );
}

#[test]
fn test_email_parser_quoted_printable() {
    let input = r#"<=21-- SEPARATOR  -->
   <tr>
    <td style=3D=22padding-left: 10px;padding-right: 10px;background-color:=
 =23f3f5fa;=22>
     <table width=3D=22100%=22 cellspacing=3D=220=22 cellpadding=3D=220=22 =
border=3D=220=22>
      <tr>
       <td style=3D=22height:5px;background-color: =23f3f5fa;=22>&nbsp;</td>
      </tr>
     </table>
    </td>
   </tr>"#;
    assert_eq!(
        quoted_printable_bytes(input.as_bytes())
            .as_ref()
            .map(|(_, b)| unsafe { std::str::from_utf8_unchecked(b) }),
        Ok(r#"<!-- SEPARATOR  -->
   <tr>
    <td style="padding-left: 10px;padding-right: 10px;background-color: #f3f5fa;">
     <table width="100%" cellspacing="0" cellpadding="0" border="0">
      <tr>
       <td style="height:5px;background-color: #f3f5fa;">&nbsp;</td>
      </tr>
     </table>
    </td>
   </tr>"#)
    );
}

#[test]
fn test_email_parser_msg_id() {
    let s = "Message-ID: <1234@local.machine.example>\r\n";
    let (rest, (_header_name, value)) = headers::header(s.as_bytes()).unwrap();
    assert!(rest.is_empty());
    let a = msg_id(value).unwrap().1;
    assert_eq!(a, "<1234@local.machine.example>");
    let s = "Message-ID:              <testabcd.1234@silly.test>\r\n";
    let (rest, (_header_name, value)) = headers::header(s.as_bytes()).unwrap();
    assert!(rest.is_empty());
    let b = msg_id(value).unwrap().1;
    assert_eq!(b, "<testabcd.1234@silly.test>");
    let s = "References: <1234@local.machine.example>\r\n";
    let (rest, (_header_name, value)) = headers::header(s.as_bytes()).unwrap();
    assert!(rest.is_empty());
    assert_eq!(&msg_id_list(value).unwrap().1, std::slice::from_ref(&a));
    let s = "References: <1234@local.machine.example> <3456@example.net>\r\n";
    let (rest, (_header_name, value)) = headers::header(s.as_bytes()).unwrap();
    assert!(rest.is_empty());
    let s = b"<3456@example.net>";
    let c = msg_id(s).unwrap().1;
    assert_eq!(&msg_id_list(value).unwrap().1, &[a, c]);
}

#[test]
fn test_email_parser_dates_date_new() {
    let s = b"Thu, 31 Aug 2017 13:43:37 +0000 (UTC)";
    let _s = b"Thu, 31 Aug 2017 13:43:37 +0000";
    let __s = b"=?utf-8?q?Thu=2C_31_Aug_2017_13=3A43=3A37_-0000?=";
    assert_eq!(rfc5322_date(s).unwrap(), rfc5322_date(_s).unwrap());
    assert_eq!(rfc5322_date(_s).unwrap(), rfc5322_date(__s).unwrap());
    let val = b"Fri, 23 Dec 0001 21:20:36 -0800 (PST)";
    assert_eq!(rfc5322_date(val).unwrap(), 0);
    let val = b"Wed Sep  9 00:27:54 2020";
    assert_eq!(rfc5322_date(val).unwrap(), 1599611274);
}

#[test]
fn test_email_parser_comment() {
    let s = b"(recursive (comment) block)";
    assert_eq!(comment(s), Ok((&b""[..], ())));
}

#[test]
fn test_email_parser_cfws() {
    let s = r#"This
 is a test"#;
    assert_eq!(&unstructured(s.as_bytes()).unwrap(), "This is a test",);

    assert_eq!(&unstructured(s.as_bytes()).unwrap(), "This is a test",);
    let s = "this is\n\ta folded name";
    assert_eq!(
        &unstructured(s.as_bytes()).unwrap(),
        "this is\ta folded name",
    );
}

#[test]
fn test_email_parser_phrase2() {
    let s = b"\"Jeffrey \\\"fejj\\\" Stedfast\""; // <fejj@helixcode.com>"
    assert_eq!(to_str!(&phrase2(s).unwrap().1), "Jeffrey \"fejj\" Stedfast");
}

#[test]
fn test_email_parser_rfc_2369_list() {
    let s = r#"List-Help: <mailto:list@host.com?subject=help> (List Instructions)
List-Help: <mailto:list-manager@host.com?body=info>
List-Help: <mailto:list-info@host.com> (Info about the list)
List-Help: <http://www.host.com/list/>, <mailto:list-info@host.com>
List-Help: <ftp://ftp.host.com/list.txt> (FTP),
    <mailto:list@host.com?subject=help>
List-Post: <mailto:list@host.com>
List-Post: <mailto:moderator@host.com> (Postings are Moderated)
List-Post: <mailto:moderator@host.com?subject=list%20posting>
List-Post: NO (posting not allowed on this list)
List-Archive: <mailto:archive@host.com?subject=index%20list>
List-Archive: <ftp://ftp.host.com/pub/list/archive/>
List-Archive: <http://www.host.com/list/archive/> (Web Archive)
"#;
    let (rest, headers) = headers::headers(s.as_bytes()).unwrap();
    assert!(rest.is_empty());
    for (_h, v) in headers {
        let (rest, _action_list) = rfc_2369_list_headers_action_list(v).unwrap();
        assert!(rest.is_empty());
    }
}

#[test]
fn test_email_parser_bytesext_trait() {
    const TO_TRIM: &[u8] = b"   foo| ";
    assert_eq!(to_str!(BytesExt::ltrim(TO_TRIM)), "foo| ");
    assert_eq!(to_str!(BytesExt::trim_start(TO_TRIM)), "foo| ");
    assert_eq!(to_str!(BytesExt::rtrim(TO_TRIM)), "   foo|");
    assert_eq!(to_str!(BytesExt::trim_end(TO_TRIM)), "   foo|");
    assert_eq!(to_str!(BytesExt::trim(TO_TRIM)), "foo|");
    const TO_REPLACE: &[u8] = b"  \t \t\n \t ";
    assert_eq!(BytesExt::find(TO_REPLACE, b"\t"), Some(2));
    assert_eq!(BytesExt::find(&TO_REPLACE[3..], b"\t"), Some(1));
    assert_eq!(BytesExt::find(&TO_REPLACE[5..], b"\t"), Some(2));
    assert_eq!(BytesExt::find(&TO_REPLACE[8..], b"\t"), None);
    assert_eq!(BytesExt::rfind(TO_REPLACE, b"\t"), Some(7));
    assert_eq!(
        to_str!(&BytesExt::replace(TO_REPLACE, b"\t", b"\0")),
        "  \0 \0\n \0 "
    );
    assert_eq!(
        to_str!(&BytesExt::replace(TO_REPLACE, b"\n", b"\0")),
        "  \t \t\0 \t "
    );
    assert_eq!(
        to_str!(&BytesExt::replace(TO_REPLACE, b" ", b"")),
        "\t\t\n\t"
    );
    assert_eq!(
        to_str!(&BytesExt::replace(TO_REPLACE, b"  ", b"")),
        "\t \t\n \t "
    );
    assert_eq!(
        to_str!(&BytesExt::replace(TO_REPLACE, b"X", b"\0")),
        to_str!(TO_REPLACE)
    );
    assert!(!BytesExt::is_quoted(TO_REPLACE));
    assert!(BytesExt::is_quoted(b"\"aaa\"".as_ref()));
}

// C8a: the multipart boundary twin loops (`multipart_parts` and `parts`,
// i.e. `parts_f`) must always terminate and never panic on hostile input
// (CWE-835 non-termination, CWE-1287 out-of-bounds/underflow). Regression
// tests promoted from the audit scratch examples `melib/examples/{c8_hang,
// c8a_oob}.rs`.

const C8A_BOUNDARY: &[u8] = b"BOUND";

/// Run `f` on a worker thread bounded by `limit`, mirroring the
/// `assert_completes_within` harness of the connection size-cap tests: a
/// parser hang fails the test deterministically within `limit` instead of
/// wedging the test binary; a panic inside `f` is surfaced as this test's
/// failure. On timeout the worker is leaked (a hung parser thread cannot be
/// killed safely); acceptable in a failing test since the process exits
/// right after.
fn assert_completes_within<T: Send + 'static>(
    limit: std::time::Duration,
    label: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    let _worker = std::thread::Builder::new()
        .name(format!("c8a_{label}"))
        .spawn(move || {
            let _ = sender.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        })
        .expect("failed to spawn C8a worker thread");
    match receiver.recv_timeout(limit) {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!(
            "{label}: parser did not complete within {limit:?} \
             (C8a/CWE-835 non-termination regression)"
        ),
    }
}

#[test]
fn test_multipart_parts_first_loop_terminates() {
    // Pre-fix: a boundary occurrence NOT preceded by `--` made the first
    // loop slice to the exact same position forever (100% CPU hang). The
    // canonical 113-byte repro mail (headers + first hang form) lives in
    // `meli/tests/test_c8a_parts_poc.rs`; here the bodies are fed directly
    // to both twin loops.
    let hang_forms: &[(&[u8], bool)] = &[
        // (body, parts() outcome): the alt fallback needs a `--` to skip to;
        // without one parts() errors out too.
        (b"\r\nBOUND\r\nmore text\r\n--BOUND--\r\n", true),
        (b"xxBOUND", false),
        (b"body text\r\nBOUND\r\n--BOUND--\r\n", true),
    ];
    for (body, parts_ok) in hang_forms {
        let multipart = assert_completes_within(
            std::time::Duration::from_secs(10),
            "multipart_parts hang form",
            || attachments::multipart_parts(body, C8A_BOUNDARY),
        );
        // Break without a dash-boundary -> the second loop's `--` prefix
        // check rejects the same occurrence: graceful parse failure.
        multipart.unwrap_err();
        let parts = assert_completes_within(
            std::time::Duration::from_secs(10),
            "parts_f hang form",
            || attachments::parts(body, C8A_BOUNDARY),
        );
        // parts_f errors out; the `parts()` alt fallback then yields an
        // empty parts list (or Err when there is no `--` to skip to), so
        // the hostile mail degrades to "no parts".
        if *parts_ok {
            assert_eq!(
                parts.expect("parts() fallback must accept hang forms").1,
                Vec::<&[u8]>::new()
            );
        } else {
            parts.unwrap_err();
        }
    }
    // Control forms that already terminate (Err paths), must stay fast:
    attachments::multipart_parts(b"BOUND", C8A_BOUNDARY).unwrap_err();
    attachments::parts(b"BOUND", C8A_BOUNDARY).unwrap_err();
    attachments::multipart_parts(b"--BOUN", C8A_BOUNDARY).unwrap_err();
    attachments::parts(b"--BOUN", C8A_BOUNDARY).unwrap_err();
}

#[test]
fn test_multipart_parts_truncated_forms_no_panic() {
    // Three panic classes, each reachable in BOTH twin loops pre-fix:
    // (a) body ends exactly at a dash-boundary with EOF (no CRLF, no
    //     closing `--`): `input[0]` out of bounds after consuming
    //     `--<boundary>`. `--BOUND\r\n` and `--BOUND--` never panicked.
    // (b) empty part (no line ending between the first boundary's CRLF and
    //     the next boundary): `end - 3` underflow with end == 2.
    // (c) part content starting with the boundary bytes: `&input[end - 2..end]`
    //     underflow with end < 2.
    let panic_forms: &[(&[u8], &str)] = &[
        (b"--BOUND" as &[u8], "(a) bare dash-boundary at EOF"),
        (b"x\r\n--BOUND", "(a) preamble then dash-boundary at EOF"),
        (
            b"preamble\r\n--BOUND",
            "(a) preamble then dash-boundary at EOF",
        ),
        (
            b"--BOUND\r\n--BOUND--\r\n",
            "(b) empty part before closing boundary",
        ),
        (
            b"--BOUND\r\n--BOUND--",
            "(b) empty part, truncated epilogue",
        ),
        (
            b"--BOUND\r\nBOUND-prefix line\r\n--BOUND--\r\n",
            "(c) part content starts with boundary bytes",
        ),
    ];
    for (body, label) in panic_forms {
        for f in [
            (format!("multipart_parts {label}"), 0),
            (format!("parts_f {label}"), 1),
        ] {
            let (name, which) = f;
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match which {
                0 => {
                    let _ = attachments::multipart_parts(body, C8A_BOUNDARY);
                }
                _ => {
                    let _ = attachments::parts(body, C8A_BOUNDARY);
                }
            }));
            if let Err(payload) = &r {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "non-string payload".into());
                eprintln!("[{name}]: PANIC: {msg}");
            }
            assert!(r.is_ok(), "{name}: panicked (C8a/CWE-1287 regression)");
        }
    }
    // Must-not-panic family (never panicked pre-fix; behavior freeze):
    let calm_forms: &[&[u8]] = &[
        b"--BOUND\r\n",
        b"--BOUND--",
        b"--BOUND\r\n--BOUND",
        b"BOUND",
        b"--BOUN",
    ];
    for body in calm_forms {
        let _ = attachments::multipart_parts(body, C8A_BOUNDARY);
        let _ = attachments::parts(body, C8A_BOUNDARY);
    }
}

#[test]
fn test_multipart_parts_hostile_outcomes() {
    // Pin post-fix outcomes for the hostile forms so regressions cannot
    // hide behind a mere "did not panic" (misleading-success class).
    // (a) dash-boundary at EOF -> Err; parts() falls back to empty list.
    attachments::multipart_parts(b"x\r\n--BOUND", C8A_BOUNDARY).unwrap_err();
    assert_eq!(
        attachments::parts(b"x\r\n--BOUND", C8A_BOUNDARY)
            .expect("parts() fallback must accept truncated boundary")
            .1,
        Vec::<&[u8]>::new()
    );
    // (b) empty part -> one empty part, not a panic.
    assert_eq!(
        attachments::parts(b"--BOUND\r\n--BOUND--\r\n", C8A_BOUNDARY)
            .unwrap()
            .1,
        vec![b"" as &[u8]]
    );
    assert_eq!(
        attachments::parts(b"--BOUND\r\n\r\n--BOUND--\r\n", C8A_BOUNDARY)
            .unwrap()
            .1,
        vec![b"" as &[u8]]
    );
    // (c) boundary-prefixed part content -> parts_f Err, fallback empty.
    assert_eq!(
        attachments::parts(b"--BOUND\r\nBOUND-x\r\n--BOUND--\r\n", C8A_BOUNDARY)
            .expect("parts() fallback must accept boundary-prefixed content")
            .1,
        Vec::<&[u8]>::new()
    );
    attachments::multipart_parts(b"--BOUND\r\nBOUND-x\r\n--BOUND--\r\n", C8A_BOUNDARY).unwrap_err();
}

#[test]
fn test_multipart_parts_clean_two_part_freeze() {
    // Behavior freeze: regular two-part multiparts must parse to the exact
    // same part bytes as before the C8a hardening.
    let lf_body: &[u8] = b"preamble\n--BOUND\nContent-Type: text/plain\n\nfirst \
                           part\n--BOUND\nContent-Type: text/plain\n\nsecond \
                           part\n--BOUND--\n";
    let expected: Vec<&[u8]> = vec![
        b"Content-Type: text/plain\n\nfirst part".as_slice(),
        b"Content-Type: text/plain\n\nsecond part".as_slice(),
    ];

    let (rest, parts) = attachments::parts(lf_body, C8A_BOUNDARY).unwrap();
    assert_eq!(parts, expected);
    assert_eq!(rest, b"--\n");

    let (rest, builders) = attachments::multipart_parts(lf_body, C8A_BOUNDARY).unwrap();
    let rendered: Vec<&[u8]> = builders
        .iter()
        .map(|sb| &lf_body[sb.offset..sb.offset + sb.length])
        .collect();
    assert_eq!(rendered, expected);
    assert_eq!(rest, b"--\n");

    // CRLF variant: parts_f (via parts()) continues after a `\r\n`
    // terminator and yields both parts byte-identically. NOTE (pre-existing
    // twin asymmetry, deliberately frozen): multipart_parts' second loop
    // breaks after the first part for CRLF bodies; not a C8a defect.
    let crlf_body: &[u8] = b"preamble\r\n--BOUND\r\nContent-Type: text/plain\r\n\r\nfirst \
                             part\r\n--BOUND\r\nContent-Type: text/plain\r\n\r\nsecond \
                             part\r\n--BOUND--\r\n";
    let (rest, parts) = attachments::parts(crlf_body, C8A_BOUNDARY).unwrap();
    assert_eq!(
        parts,
        vec![
            b"Content-Type: text/plain\r\n\r\nfirst part".as_slice(),
            b"Content-Type: text/plain\r\n\r\nsecond part".as_slice(),
        ]
    );
    assert_eq!(rest, b"--\r\n");

    let (rest, builders) = attachments::multipart_parts(crlf_body, C8A_BOUNDARY).unwrap();
    let rendered: Vec<&[u8]> = builders
        .iter()
        .map(|sb| &crlf_body[sb.offset..sb.offset + sb.length])
        .collect();
    assert_eq!(
        rendered,
        vec![b"Content-Type: text/plain\r\n\r\nfirst part".as_slice()]
    );
    // Rest = everything after the second boundary line's `BOUND` token.
    assert_eq!(
        rest,
        b"\r\nContent-Type: text/plain\r\n\r\nsecond part\r\n--BOUND--\r\n".as_slice()
    );
}
