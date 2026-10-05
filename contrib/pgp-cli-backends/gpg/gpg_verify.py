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

if len(sys.argv) <= 1:
    print(sys.argv, "needs two arguments, the signature file and the signed data file")
    sys.exit(1)

sig_file = open(sys.argv[1])
signed_file = open(sys.argv[2])

auto_key_locate = "local"
if "AUTO_KEY_LOCATE" in os.environ:
    auto_key_locate = os.environ["AUTO_KEY_LOCATE"]

# TODO
is_cleartext = "CLEARTEXT" in os.environ

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
            f"-&{signed_file.fileno()}",
        ],
        timeout=2,
        check=False,
        capture_output=True,
        text=False,
        pass_fds=[status_fd[1], logger_fd[1], sig_file.fileno(), signed_file.fileno()],
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

err = re.compile(err, flags=re.M).search(status)
summary_match = re.compile(summary, flags=re.M).search(status)

# CVE-2018-12020 (SigSpoof 1): GnuPG < 2.2.8 wrote the raw plaintext
# packet filename into the --status-fd stream, so attacker bytes could
# arrive as forged "[GNUPG:] GOODSIG/VALIDSIG/TRUST_* ..." lines. The
# verdict must therefore never rest on the parseable status text alone:
# gpg's exit status is the authoritative verdict (0 == every signature
# verified), and only an explicit GOODSIG summary on a successful exit
# affirms. ERRSIG, a failing exit, or a missing summary are reported as
# error statuses so the client fails closed.
if err or summary_match is None or s.returncode != 0:
    if err:
        detail = err.group(0)
    elif summary_match is not None:
        # gpg itself reported a failing verdict (e.g. BADSIG or a mixed
        # multi-signature run whose exit status is non-zero).
        detail = summary_match.group(0)
    else:
        detail = f"gpg exited with status {s.returncode} without a signature summary"
    print(json.dumps(detail))
else:
    fingerprint = re.compile(fpr, flags=re.M).search(status).group("fingerprint")
    trust = re.compile(trust, flags=re.M).search(status)
    trust_level = trust.group("trust")
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
    if summary == "GOODSIG":
        print(
            json.dumps(
                [
                    {
                        "summary": summary_val,
                        "cert": {
                            "keyid": fingerprint,
                            # The CLI backend contract requires the status to be
                            # explicit: "OK" affirms a good signature, any other
                            # string is an error message, and an absent status is
                            # treated by meli as "not reported" and fails closed
                            # (CVE-2007-1265: KMail showed mails as signed when
                            # it failed to read GnuPG's status output).
                            "status": "OK",
                        },
                        "validity": trust_level,
                        "cleartext": is_cleartext,
                    }
                ]
            )
        )
    else:
        # Exit status 0 with a non-affirming summary (BADSIG, REVKEYSIG,
        # EXPKEYSIG, EXPSIG): report the verdict as an error status so the
        # client renders a bad signature instead of a good one.
        print(
            json.dumps(
                [
                    {
                        "summary": summary_val,
                        "cert": {
                            "keyid": fingerprint,
                            "status": summary_match.group(0),
                        },
                        "validity": trust_level,
                        "cleartext": is_cleartext,
                    }
                ]
            )
        )
