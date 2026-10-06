#!/usr/bin/env python3
# SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
# Copyright 2026 Manos Pitsidianakis

import subprocess
import sys
import json
import re
import os

status_fd = os.pipe()
logger_fd = os.pipe()

# Cleartext signature mode: meli's verify_cleartext invokes the script with
# a single file argument and the CLEARTEXT marker in the environment.
is_cleartext = "CLEARTEXT" in os.environ

if is_cleartext:
    # Cleartext signatures: the signed text and its embedded armor are one
    # file (meli's verify_cleartext invokes the script with a single
    # argument). gpg --verify also takes the single file in this form.
    if len(sys.argv) != 2:
        print(sys.argv, "needs one argument, the cleartext signed file")
        sys.exit(1)
    sig_file = open(sys.argv[1])
    signed_file = None
else:
    if len(sys.argv) <= 2:
        print(sys.argv, "needs two arguments, the signature file and the signed data file")
        sys.exit(1)
    sig_file = open(sys.argv[1])
    signed_file = open(sys.argv[2])

auto_key_locate = "local"
if "AUTO_KEY_LOCATE" in os.environ:
    auto_key_locate = os.environ["AUTO_KEY_LOCATE"]

try:
    s = subprocess.run(
        [
            "gpg",
            "--enable-special-filenames",
            f"--auto-key-locate={auto_key_locate}",
            "--disable-dirmngr",
            "--batch",
            "--status-fd",
            str(status_fd[1]),
            "--logger-fd",
            str(logger_fd[1]),
            "--no-tty",
            "--charset=utf8",
            "--enable-progress-filter",
            "--exit-on-status-write-error",
            "--verify",
            "--",
            f"-&{sig_file.fileno()}",
        ]
        + ([f"-&{signed_file.fileno()}"] if signed_file is not None else []),
        timeout=2,
        check=False,
        capture_output=True,
        text=False,
        pass_fds=(
            [status_fd[1], logger_fd[1], sig_file.fileno()]
            + ([signed_file.fileno()] if signed_file is not None else [])
        ),
    )
except subprocess.CalledProcessError as exc:
    os.close(status_fd[1])
    os.close(logger_fd[1])
    status = os.fdopen(status_fd[0])
    status = status.read()
    logger = os.fdopen(logger_fd[0])
    logger = logger.read()
    print(
        json.dumps(
            {
                "returncode": exc.returncode,
                "cmd": exc.cmd,
                "stdout": exc.stdout.decode("utf-8"),
                "stderr": exc.stderr.decode("utf-8"),
                "status_fd": status,
                "logger_fd": logger,
            }
        )
    )
    sys.exit(1)

os.close(status_fd[1])
os.close(logger_fd[1])
status = os.fdopen(status_fd[0])
status = status.read()
logger = os.fdopen(logger_fd[0])
logger = logger.read()

fpr = r"^\[GNUPG:\] KEY_CONSIDERED (?P<fingerprint>[^ ]*).*$"
trust = r"^\[GNUPG:\] TRUST_(?P<trust>[^ ]*) (?P<error_token>\d\d*)(?: (?P<validation_model>\w+))?.*$"
summary = r"^\[GNUPG:\] (?P<summary>(?:GOODSIG|BADSIG|REVKEYSIG|EXPKEYSIG|EXPSIG)) (?P<keyid>\w+)"
err = r"^\[GNUPG:\] ERRSIG (?P<keyid>\w+) (?P<pkalgo>\w+) (?P<hashalgo>\w+) (?P<sig_class>\w+) (?P<time>(?:(?:\d{4}-\d{2}-\d{2})|(:?\d+))) (?P<rc>\d+)"

fpr_re = re.compile(fpr, flags=re.M)
trust_re = re.compile(trust, flags=re.M)
summary_re = re.compile(summary, flags=re.M)
err = re.compile(err, flags=re.M).search(status)

# CVE-2018-12020 (SigSpoof 1): GnuPG < 2.2.8 wrote the raw plaintext
# packet filename into the --status-fd stream, so attacker bytes could
# arrive as forged "[GNUPG:] GOODSIG/VALIDSIG/TRUST_* ..." lines. The
# verdict must therefore never rest on the parseable status text alone:
# gpg's exit status is the authoritative verdict (0 == every signature
# verified), and only an explicit GOODSIG summary on a successful exit
# affirms. ERRSIG, a failing exit, or a missing summary are reported as
# error statuses so the client fails closed.
#
# CVE-2018-12019 (SigSpoof 2): a signature file may carry more than one
# signature packet — a valid one beside a forged one. GnuPG reports one
# status segment per signature (each opening with a NEWSIG line) and its
# exit status is 0 only when every signature verified. Reporting only the
# first signature of the stream would recreate the Enigmail < 2.0.7
# confusion surface where the user is shown a single signature and told
# it is the message's signature: the stream is therefore split into
# per-signature segments and EVERY segment is reported, and a run where
# any segment lacks a verdict is refused outright.

lines = status.splitlines()

newsig_indices = [
    i for (i, line) in enumerate(lines) if line == "[GNUPG:] NEWSIG"
]
if newsig_indices:
    # One segment per signature: the status lines from each NEWSIG up to
    # the next one. Anything before the first NEWSIG (e.g. PROGRESS
    # lines) is engine bookkeeping, not a signature verdict.
    segments = [
        lines[(start + 1) : (newsig_indices[(idx + 1)] if idx + 1 < len(newsig_indices) else len(lines))]
        for (idx, start) in enumerate(newsig_indices)
    ]
else:
    # No NEWSIG markers (very old engines): the whole stream is the one
    # signature's segment.
    segments = [lines] if lines else []

# Per-signature fields: the summary line (GOODSIG/BADSIG/...), the full
# fingerprint from the segment's own KEY_CONSIDERED line and the segment's
# own TRUST_* validity.
fields = []
for segment in segments:
    text = "\n".join(segment)
    fields.append(
        (
            summary_re.search(text),
            fpr_re.search(text),
            trust_re.search(text),
        )
    )

first_summary = next((m for (m, _, _) in fields if m is not None), None)
first_non_affirming = next(
    (m for (m, _, _) in fields if m is not None and m.group("summary") != "GOODSIG"),
    None,
)

if (
    err
    or s.returncode != 0
    or first_summary is None
    or any(m is None for (m, _, _) in fields)
):
    if err:
        detail = err.group(0)
    elif first_non_affirming is not None:
        # gpg itself reported a failing verdict (e.g. BADSIG or a mixed
        # multi-signature run whose exit status is non-zero); name the
        # signature that failed, not the first one that happened to pass.
        detail = first_non_affirming.group(0)
    elif first_summary is not None:
        detail = first_summary.group(0)
    else:
        detail = f"gpg exited with status {s.returncode} without a signature summary"
    print(json.dumps(detail))
else:
    # Affirming run: gpg exited 0 and every reported segment carries a
    # verdict. Emit every signature of the multi-signature stream, each
    # with its own fingerprint and validity; a non-affirming summary on a
    # successful exit (a text/exit contradiction gpg itself does not
    # produce) keeps the structured error-status contract and fails
    # closed on the meli side.
    out = []
    for (summary_match, fpr_match, trust_match) in fields:
        summary = summary_match.group("summary")
        summary_val = []

        if summary == "GOODSIG":
            pass
        elif summary == "BADSIG":
            summary_val.append("RED")
        elif summary == "REVKEYSIG":
            summary_val.append("KEY_REVOKED")
        elif summary == "EXPKEYSIG":
            summary_val.append("KEY_EXPIRED")
        elif summary == "EXPSIG":
            summary_val.append("SIG_EXPIRED")
        out.append(
            {
                "summary": summary_val,
                "cert": {
                    "keyid": (
                        fpr_match.group("fingerprint")
                        if fpr_match is not None
                        else summary_match.group("keyid")
                    ),
                    # The CLI backend contract requires the status to be
                    # explicit: "OK" affirms a good signature, any other
                    # string is an error message, and an absent status is
                    # treated by meli as "not reported" and fails closed
                    # (CVE-2007-1265: KMail showed mails as signed when
                    # it failed to read GnuPG's status output).
                    "status": "OK" if summary == "GOODSIG" else summary_match.group(0),
                },
                "validity": (
                    trust_match.group("trust") if trust_match is not None else "UNKNOWN"
                ),
                "cleartext": is_cleartext,
            }
        )
    print(json.dumps(out))
