//! `store/LOCK` refuses a second store/server on the same directory.

use annotary_store::{Store, StoreError};

// spec: S01-02
#[test]
fn second_open_of_the_same_store_is_refused_while_the_first_lives() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let first = Store::open(&path).unwrap();
    assert!(path.join("LOCK").exists());
    match Store::open(&path) {
        Err(StoreError::Locked { path: p, pid }) => {
            assert_eq!(p, path);
            assert_eq!(pid, Some(std::process::id()));
        }
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("second open must be refused"),
    }
    drop(first);
    let again = Store::open(&path).unwrap();
    assert!(again.dir().join("LOCK").exists(), "LOCK stays; the flock is what matters");
}

// spec: S01-02
#[test]
fn a_second_in_process_server_on_the_same_store_is_refused() {
    let ts = annotary::testing::TestServer::start();
    let store_dir = ts.data_dir.join("store");
    let err = Store::open(&store_dir).map(|_| ()).expect_err("locked");
    assert!(matches!(err, StoreError::Locked { .. }), "{err}");
    let cli: annotary::exit::CliError = err.into();
    assert_eq!(cli.exit, annotary::exit::Exit::Refused);
    assert!(cli.text_line().contains("locked by another annotary serve"), "{}", cli.text_line());
}
