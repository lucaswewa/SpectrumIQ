//! The accounts (ADR-0065): users added, verified, changed and removed, kept in their file and
//! read again when another process changes it; argon2id at OWASP's parameters; the last admin
//! kept; names and passwords checked. REQ-SEC-001.

use siq_auth::{Role, UserError, UserStore};

fn store() -> (tempfile::TempDir, UserStore) {
    let folder = tempfile::tempdir().unwrap();
    let store = UserStore::open(folder.path().join("settings/users.json")).unwrap();
    (folder, store)
}

#[test]
fn users_are_added_verified_changed_and_removed() {
    let (_folder, store) = store();
    assert!(store.is_empty().unwrap());
    store.add("ada", "correct horse", Role::Admin).unwrap();
    store.add("bob", "battery staple", Role::Viewer).unwrap();
    let names: Vec<String> = store.list().unwrap().into_iter().map(|u| u.name).collect();
    assert_eq!(names, ["ada", "bob"]);

    let bob = store.verify("bob", "battery staple").unwrap().unwrap();
    assert_eq!(bob.role, Role::Viewer);
    assert!(store.verify("bob", "battery stapler").unwrap().is_none());
    assert!(store.verify("eve", "battery staple").unwrap().is_none());

    store.set_password("bob", "a new staple").unwrap();
    assert!(store.verify("bob", "battery staple").unwrap().is_none());
    assert!(store.verify("bob", "a new staple").unwrap().is_some());
    assert_eq!(
        store.set_role("bob", Role::Operator).unwrap().role,
        Role::Operator
    );
    store.remove("bob").unwrap();
    assert!(store.get("bob").unwrap().is_none());
    assert!(matches!(store.remove("bob"), Err(UserError::Unknown(_))));
    assert!(matches!(
        store.add("ada", "whatever else", Role::Viewer),
        Err(UserError::Exists(_))
    ));
}

#[test]
fn the_file_keeps_hashes_at_owasp_s_parameters_and_no_passwords() {
    let (_folder, store) = store();
    store.add("ada", "correct horse", Role::Admin).unwrap();
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(!text.contains("correct horse"));
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    let hash = file["users"][0]["hash"].as_str().unwrap();
    // Argon2id, version 0x13, 19 MiB, 2 passes, 1 lane: OWASP's first recommendation.
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{hash}"
    );
    let [salt, output] = hash.rsplitn(3, '$').take(2).collect::<Vec<_>>()[..] else {
        panic!("{hash}")
    };
    // A 16-byte salt and a 32-byte hash, in base64 without padding.
    assert_eq!((salt.len(), output.len()), (43, 22), "{hash}");
    // The same password, again: another salt, so another hash.
    store.add("bob", "correct horse", Role::Viewer).unwrap();
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    assert_ne!(file["users"][0]["hash"], file["users"][1]["hash"]);
}

#[test]
fn another_process_s_changes_are_read_again() {
    let (_folder, store) = store();
    store.add("ada", "correct horse", Role::Admin).unwrap();
    // The CLI, say, on the same file.
    let cli = UserStore::open(store.path()).unwrap();
    cli.add("bob", "battery staple", Role::Operator).unwrap();
    assert_eq!(store.get("bob").unwrap().unwrap().role, Role::Operator);
    cli.remove("bob").unwrap();
    assert!(store.verify("bob", "battery staple").unwrap().is_none());
}

#[test]
fn the_last_admin_stays_and_bad_names_and_short_passwords_are_refused() {
    let (_folder, store) = store();
    store.add("ada", "correct horse", Role::Admin).unwrap();
    assert!(matches!(store.remove("ada"), Err(UserError::LastAdmin)));
    assert!(matches!(
        store.set_role("ada", Role::Operator),
        Err(UserError::LastAdmin)
    ));
    store.add("root", "correct horse", Role::Admin).unwrap();
    store.set_role("ada", Role::Operator).unwrap();
    for name in ["", "ada lovelace", "a/b", &"x".repeat(65)] {
        let refused = store.add(name, "correct horse", Role::Viewer);
        assert!(matches!(refused, Err(UserError::Name(_))), "{name:?}");
    }
    let short = store.add("eve", "seven77", Role::Viewer);
    assert!(matches!(short, Err(UserError::Password)));
    assert_eq!("Admin".parse::<Role>().unwrap(), Role::Admin);
    assert!("root".parse::<Role>().is_err());
}
