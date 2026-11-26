//! `Store::cache_scope` (#434): one name per database file, however its path
//! is spelled, and a name of its own for every in-memory database.

use mailcrane_store::Store;

#[test]
fn every_spelling_of_one_file_has_one_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plain = dir.path().join("mail.db");
    let store = Store::open(&plain).expect("open");
    let scope = store.cache_scope().to_owned();
    // The same file through a `.` and a `..` hop: the scope a cache would be
    // keyed under when another entry point spells the path differently.
    let dotted = dir.path().join(".").join("sub").join("..").join("mail.db");
    std::fs::create_dir(dir.path().join("sub")).expect("sub");
    assert_eq!(Store::scope_of(&dotted), scope);
    assert_eq!(Store::scope_of(&plain), scope);
    // Opened under the other spelling, it is the same database.
    assert_eq!(Store::open(&dotted).expect("reopen").cache_scope(), scope);
    // A different file is a different scope.
    let other = Store::open(dir.path().join("other.db")).expect("other");
    assert_ne!(other.cache_scope(), scope);
}

#[test]
fn every_in_memory_database_has_a_scope_of_its_own() {
    let a = Store::open_in_memory().expect("a");
    let b = Store::open_in_memory().expect("b");
    let c = Store::open(":memory:").expect("c");
    let d = Store::open(":memory:").expect("d");
    let scopes = [
        a.cache_scope(),
        b.cache_scope(),
        c.cache_scope(),
        d.cache_scope(),
    ];
    for (i, x) in scopes.iter().enumerate() {
        for y in &scopes[i + 1..] {
            assert_ne!(x, y);
        }
    }
}

#[test]
fn a_file_that_does_not_exist_yet_keeps_its_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("not-yet.db");
    assert!(Store::scope_of(&missing).ends_with("not-yet.db"));
}
