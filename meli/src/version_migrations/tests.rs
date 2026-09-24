//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
// Copyright 2026 Kyle Lee
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

use rusty_fork::rusty_fork_test;

use super::*;

#[test]
fn test_version_migrations_version_map() {
    let version_map = indexmap::indexmap! {
        v0_8_8::V0_8_8_ID => Box::new(V0_8_8) as Box<dyn Version + Send + Sync + 'static>,
        v0_8_9::V0_8_9_ID => Box::new(V0_8_9) as Box<dyn Version + Send + Sync + 'static>,
    };
    assert!(
        version_map.contains_key("0.8.8"),
        "Could not access Version identifier by &str key in version map"
    );
    assert!(
        version_map.contains_key("0.8.9"),
        "Could not access Version identifier by &str key in version map"
    );
    assert!(!version_map.contains_key("0.0.0"),);
    assert!(
        version_map.contains_key(&v0_8_8::V0_8_8_ID),
        "Could not access Version identifier by VersionIdentifier key in version map"
    );
}

#[test]
fn test_version_migrations_returns_correct_migration() {
    let version_map = indexmap::indexmap! {
        v0_8_8::V0_8_8_ID => Box::new(V0_8_8) as Box<dyn Version + Send + Sync + 'static>,
        v0_8_9::V0_8_9_ID => Box::new(V0_8_9) as Box<dyn Version + Send + Sync + 'static>,
    };
    let migrations = calculate_migrations(Some("0.8.8"), &version_map);
    assert!(
        migrations.is_empty(),
        "Calculated migrations between 0.8.8 and 0.8.9 are not empty: {migrations:?}"
    );
    let migrations = calculate_migrations(None, &version_map);
    assert!(
        !migrations.is_empty(),
        "Calculated migrations between no version and 0.8.8 are empty",
    );
}

#[test]
fn test_parse_version_valid() {
    assert_eq!(parse_version("0.9.0"), Some((0, 9, 0, "")));
    assert_eq!(parse_version("0.8.8"), Some((0, 8, 8, "")));
    assert_eq!(parse_version("0.8.8-rc1"), Some((0, 8, 8, "rc1")));
    assert_eq!(parse_version("255.255.255"), Some((255, 255, 255, "")));
    assert_eq!(parse_version("0.0.0"), Some((0, 0, 0, "")));
}

#[test]
fn test_parse_version_invalid() {
    // The `.version` file contents are trimmed in `version_setup` before
    // parsing; the trailing whitespace case only locks down the strictness
    // of the parser itself.
    for s in [
        "meli-git", "", "0.8", "0.8.x", "x.8.8", "0.256.0", "1.2.3+b", "v0.8.8", "0.8.8 ",
    ] {
        assert_eq!(parse_version(s), None, "parse_version({s:?})");
    }
}

#[test]
fn test_calculate_migrations_valid_version_not_in_map() {
    let version_map = versions();
    // `0.8.7` predates every known version: the full set must be returned,
    // including the migrations of v0.8.8.
    let migrations = calculate_migrations(Some("0.8.7"), version_map);
    assert!(
        migrations
            .iter()
            .any(|(k, m)| k.as_str() == "0.8.8" && !m.is_empty()),
        "expected v0.8.8 migrations for stored version `0.8.7`, got: {migrations:?}"
    );
}

#[test]
fn test_calculate_migrations_prerelease() {
    let version_map = versions();
    // A pre-release of v0.8.8 predates the v0.8.8 release, so the
    // migrations of v0.8.8 must be included.
    let migrations = calculate_migrations(Some("0.8.8-rc1"), version_map);
    assert!(
        migrations
            .iter()
            .any(|(k, m)| k.as_str() == "0.8.8" && !m.is_empty()),
        "expected v0.8.8 migrations for stored version `0.8.8-rc1`, got: {migrations:?}"
    );
}

#[test]
fn test_calculate_migrations_unrecognized_version() {
    let version_map = versions();
    let ids = |v: Option<&str>| -> Vec<&'static str> {
        calculate_migrations(v, version_map)
            .into_iter()
            .map(|(k, _)| k.as_str())
            .collect()
    };
    // An unrecognized value must be treated like a missing one: check every
    // migration.
    assert_eq!(
        ids(Some("meli-git")),
        ids(None),
        "an unrecognized stored version must be treated like a missing one"
    );
    // A valid version newer than every known one has no migrations left to
    // perform.
    assert_eq!(ids(Some("0.9.5")), Vec::<&str>::new());
}

rusty_fork_test! {
#[test]
fn test_version_migrations_ignores_newer_version() {
    const MAX: VersionIdentifier = VersionIdentifier {
        string: "255.255.255",
        major: u8::MAX,
        minor: u8::MAX,
        patch: u8::MAX,
        pre: "",
    };
    let tempdir = tempfile::tempdir().unwrap();
    for var in [
        "MELI_CONFIG",
        "HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_CONFIG_DIRS",
        "XDG_CONFIG_HOME",
        "XDG_DATA_DIRS",
        "XDG_DATA_HOME",
    ] {
        std::env::remove_var(var);
    }
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    let version_file = version_file().unwrap();
    std::fs::write(&version_file, MAX.as_str()).unwrap();
    let config_path = tempdir.path().join("meli.toml");
    std::fs::write(&config_path,
br#"
[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password_command = "false"
"#).unwrap();

    {
        let mut stdout = vec![];
        let mut stdin = &b"y\n"[..];
        let mut stdin_buf_reader = std::io::BufReader::new(&mut stdin);
        version_setup(&config_path, &mut stdout, &mut stdin_buf_reader).unwrap();
        let expected_output = format!("This version of meli, {latest}, appears to be older than the previously used one stored in the file {version_file}: {max_version}.\nCertain configuration options might not be compatible with this version, refer to release changelogs if you need to troubleshoot configuration options problems.\nUpdate .version file to make this warning go away? (CAUTION: current configuration and stored data might not be compatible with this version!!) [y/N] ", latest = LATEST.as_str(), version_file = version_file.display(), max_version = MAX.as_str());
        assert_eq!(String::from_utf8_lossy(&stdout).as_ref(), &expected_output);
        assert_eq!(stdin_buf_reader.buffer(), b"");
        let updated_version =
            std::fs::read_to_string(&version_file).unwrap();
        assert_eq!(updated_version.trim(), LATEST.as_str());
    }
    {
        use std::io::BufRead;

        let mut stdout = vec![];
        let mut stdin = &b"N\n"[..];
        let mut stdin_buf_reader = std::io::BufReader::new(&mut stdin);

        version_setup(&config_path, &mut stdout, &mut stdin_buf_reader).unwrap();
        assert_eq!(String::from_utf8_lossy(&stdout).as_ref(), "");
        assert_eq!(stdin_buf_reader.fill_buf().unwrap(), b"N\n");
    }
    {
        std::fs::write(&version_file, MAX.as_str()).unwrap();
        let mut stdout = vec![];
        let mut stdin = &b"n\n"[..];
        let mut stdin_buf_reader = std::io::BufReader::new(&mut stdin);
        version_setup(&config_path, &mut stdout, &mut stdin_buf_reader).unwrap();
        let expected_output = format!("This version of meli, {latest}, appears to be older than the previously used one stored in the file {version_file}: {max_version}.\nCertain configuration options might not be compatible with this version, refer to release changelogs if you need to troubleshoot configuration options problems.\nUpdate .version file to make this warning go away? (CAUTION: current configuration and stored data might not be compatible with this version!!) [y/N] ", latest = LATEST.as_str(), version_file = version_file.display(), max_version = MAX.as_str());
        assert_eq!(String::from_utf8_lossy(&stdout).as_ref(), &expected_output);
        assert_eq!(stdin_buf_reader.buffer(), b"");
        let stored_version =
            std::fs::read_to_string(&version_file).unwrap();
        assert_eq!(stored_version.trim(), MAX.as_str());
    }
}
}

rusty_fork_test! {
#[test]
fn test_version_migrations_ignores_unrecognized_version() {
    let tempdir = tempfile::tempdir().unwrap();
    for var in [
        "MELI_CONFIG",
        "HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_CONFIG_DIRS",
        "XDG_CONFIG_HOME",
        "XDG_DATA_DIRS",
        "XDG_DATA_HOME",
    ] {
        std::env::remove_var(var);
    }
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    let version_file = version_file().unwrap();
    std::fs::write(&version_file, "meli-git").unwrap();
    let config_path = tempdir.path().join("meli.toml");
    std::fs::write(&config_path,
br#"
[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password_command = "false"
"#).unwrap();

    // No `addressbook` data file exists, so no migration is applicable: the
    // unrecognized value must self-heal without any interactive question.
    let mut stdout = vec![];
    let mut stdin = &b""[..];
    let mut stdin_buf_reader = std::io::BufReader::new(&mut stdin);
    version_setup(&config_path, &mut stdout, &mut stdin_buf_reader).unwrap();
    let expected_output = format!(
        "warning: version file {} contains an unrecognized value {:?}; treating it as predating \
         {} and checking for applicable migrations\n",
        version_file.display(),
        "meli-git",
        LATEST.as_str()
    );
    assert_eq!(String::from_utf8_lossy(&stdout).as_ref(), &expected_output);
    // No interactive question was asked.
    assert_eq!(stdin_buf_reader.buffer(), b"");
    // The unrecognized value was replaced with the current version.
    let updated_version = std::fs::read_to_string(&version_file).unwrap();
    assert_eq!(updated_version.trim(), LATEST.as_str());
}
}

rusty_fork_test! {
#[test]
fn test_version_migrations_unrecognized_version_performs_migrations() {
    let tempdir = tempfile::tempdir().unwrap();
    for var in [
        "MELI_CONFIG",
        "HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_CONFIG_DIRS",
        "XDG_CONFIG_HOME",
        "XDG_DATA_DIRS",
        "XDG_DATA_HOME",
    ] {
        std::env::remove_var(var);
    }
    std::env::set_var("HOME", tempdir.path());
    std::env::set_var("XDG_DATA_HOME", tempdir.path());
    let version_file = version_file().unwrap();
    std::fs::write(&version_file, "meli-git").unwrap();
    let config_path = tempdir.path().join("meli.toml");
    std::fs::write(&config_path,
br#"
[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password_command = "false"
"#).unwrap();
    // An `addressbook` data file exists, so the v0.8.8 AddressbookRename
    // migration is applicable.
    let addressbook = tempdir.path().join("meli").join("imap").join("addressbook");
    std::fs::create_dir_all(addressbook.parent().unwrap()).unwrap();
    std::fs::write(&addressbook, "addressbook contents\n").unwrap();

    let mut stdout = vec![];
    // Decline performing the migration, then accept updating the `.version`
    // file anyway.
    let mut stdin = &b"n\ny\n"[..];
    let mut stdin_buf_reader = std::io::BufReader::new(&mut stdin);
    version_setup(&config_path, &mut stdout, &mut stdin_buf_reader).unwrap();
    let expected_output = format!(
        "warning: version file {vf} contains an unrecognized value {prev:?}; treating it as \
         predating {latest} and checking for applicable migrations\nYou might need to migrate \
         your configuration data for the new version to work.\nYou can skip any changes you \
         don't want to happen and you can quit at any time.\n1 migration is about to be \
         performed:\nv0.8.8/AddressbookRename: {desc}\nPerform 1 migration? [Y/n] Update \
         .version file despite not attempting migrations? [y/N] ",
        vf = version_file.display(),
        prev = "meli-git",
        latest = LATEST.as_str(),
        desc = "The storage file for contacts, stored in the application's data folder, was \
                renamed from `addressbook` to `contacts` to better reflect its purpose."
    );
    assert_eq!(String::from_utf8_lossy(&stdout).as_ref(), &expected_output);
    assert_eq!(stdin_buf_reader.buffer(), b"");
    // The addressbook file was not touched since the migration was declined.
    assert_eq!(
        std::fs::read_to_string(&addressbook).unwrap(),
        "addressbook contents\n"
    );
    let updated_version = std::fs::read_to_string(&version_file).unwrap();
    assert_eq!(updated_version.trim(), LATEST.as_str());
}
}
