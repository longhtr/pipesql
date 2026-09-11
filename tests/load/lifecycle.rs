use super::{ROW, TempDir, config};
use pipesql::{CancellationToken, CommitResolution, Database, Error, TransactionId};
use std::fs;

#[test]
fn public_load_commit_close_reopen_and_resolve() {
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let input_path = temp.0.join("lineitem.tbl");
    fs::write(&input_path, ROW).expect("write input");
    let mut database = Database::create(&database_path, config()).expect("create");
    let resident = database.reserved_memory_bytes();
    let commit = database
        .load_lineitem(&input_path, &CancellationToken::new())
        .expect("load");
    assert_eq!(commit.generation(), 1);
    assert_eq!(database.generation(), 1);
    assert_eq!(database.reserved_memory_bytes(), resident);
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().expect("close");

    let token =
        TransactionId::from_bytes(*commit.transaction().as_bytes()).expect("reconstruct token");
    let database = Database::open(&database_path, config()).expect("reopen");
    assert_eq!(
        database.resolve_commit(token).expect("resolve"),
        CommitResolution::Durable(commit)
    );
    let mut future = *token.as_bytes();
    future[16..].copy_from_slice(&2_u64.to_le_bytes());
    assert!(matches!(
        database.resolve_commit(TransactionId::from_bytes(future).unwrap()),
        Err(Error::NotFound)
    ));
    let mut foreign = *token.as_bytes();
    foreign[0] ^= 1;
    assert!(matches!(
        database.resolve_commit(TransactionId::from_bytes(foreign).unwrap()),
        Err(Error::NotFound)
    ));
    database.close().expect("close reopened");
}

#[test]
fn token_reconstruction_validates_shape_not_issuance() {
    assert!(matches!(
        TransactionId::from_bytes([0; 24]),
        Err(Error::InvalidTransactionId)
    ));
    let mut bytes = [1; 24];
    bytes[16..].fill(0);
    assert!(matches!(
        TransactionId::from_bytes(bytes),
        Err(Error::InvalidTransactionId)
    ));
    bytes[..16].fill(0);
    bytes[16..].fill(1);
    assert!(matches!(
        TransactionId::from_bytes(bytes),
        Err(Error::InvalidTransactionId)
    ));
    assert_eq!(
        TransactionId::from_bytes([1; 24]).unwrap().as_bytes(),
        &[1; 24]
    );
}
