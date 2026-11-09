use super::*;
use std::{sync::{Arc, Barrier}, thread};

fn database() -> (tempfile::TempDir, ActionDb, gix::Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = gix::init(dir.path()).unwrap();
    let db = ActionDb::open(&repo,true).unwrap();
    db.forge_initialize().unwrap();
    (dir,db,repo)
}
fn create(id: &str) -> Request { Request::CreateIssue { operation_id:id.into(),title:"  Issue title  ".into(),body:"line1\r\nline2\r".into() } }
fn mutate(db: &ActionDb, request: &Request) -> Result<Thread> {
    request.validate()?;
    db.forge_mutate(request,&digest(&serde_json::to_vec(request)?),None,None)
}

#[test]
fn create_edit_comment_and_receipt_replay_survive_reopen() {
    let (dir,db,repo) = database();
    let original = db.forge_control(&repo,create("create")).unwrap();
    assert_eq!(original["number"],1);
    assert_eq!(original["title"],"Issue title");
    assert_eq!(original["body"],"line1\nline2\n");
    let edited = db.forge_control(&repo,Request::Edit { operation_id:"edit".into(),number:1,expected_revision:1,title:"Updated".into(),body:"new".into() }).unwrap();
    assert_eq!(edited["revision"],2);
    let comment = Request::Comment { operation_id:"comment".into(),number:1,body:"message".into() };
    let commented = db.forge_control(&repo,comment.clone()).unwrap();
    assert_eq!(commented["comment_count"],1);
    assert_eq!(db.forge_control(&repo,create("create")).unwrap(),original);
    assert_eq!(db.forge_control(&repo,comment.clone()).unwrap(),commented);
    drop(db);
    let repo = crate::git::discover_at(dir.path()).unwrap();
    let db = ActionDb::open(&repo,false).unwrap();
    assert_eq!(db.forge_control(&repo,comment).unwrap(),commented);
    assert_eq!(db.forge_get(1).unwrap().revision,3);
}

#[test]
fn operation_keys_cannot_be_reused_for_changed_requests_or_threads() {
    let (_dir,db,repo) = database();
    db.forge_control(&repo,create("key")).unwrap();
    assert!(db.forge_control(&repo,Request::CreateIssue { operation_id:"key".into(),title:"other".into(),body:"".into() }).is_err());
    assert!(db.forge_control(&repo,Request::Comment { operation_id:"key".into(),number:1,body:"hello".into() }).is_err());
    assert_eq!(db.forge_get(1).unwrap().revision,1);
}

#[test]
fn competing_expected_revision_writers_have_one_winner() {
    let (_dir,db,repo) = database();
    mutate(&db,&create("initial")).unwrap();
    let path = database_path(&repo);
    let handles: Vec<_> = (0..4).map(|_| ActionDb::open_path(&path,false).unwrap()).collect();
    let barrier = Arc::new(Barrier::new(4));
    let threads: Vec<_> = handles.into_iter().enumerate().map(|(i,db)| {
        let barrier = barrier.clone();
        thread::spawn(move || {
            let request = Request::Edit { operation_id:format!("edit-{i}"),number:1,expected_revision:1,title:format!("Writer {i}"),body:"".into() };
            barrier.wait(); mutate(&db,&request).is_ok()
        })
    }).collect();
    assert_eq!(threads.into_iter().map(|t| t.join().unwrap()).filter(|v| *v).count(),1);
    assert_eq!(db.forge_get(1).unwrap().revision,2);
    let tx = db.db.begin_read().unwrap();
    assert_eq!(tx.open_table(EVENTS).unwrap().iter().unwrap().count(),2);
    assert_eq!(tx.open_table(RECEIPTS).unwrap().iter().unwrap().count(),2);
}

#[test]
fn concurrent_comments_append_without_lost_updates_or_duplicate_retries() {
    let (_dir,db,repo) = database();
    mutate(&db,&create("initial")).unwrap();
    let path = database_path(&repo);
    let threads: Vec<_> = (0..4).map(|i| {
        let db = ActionDb::open_path(&path,false).unwrap();
        thread::spawn(move || {
            let request = Request::Comment { operation_id:format!("comment-{i}"),number:1,body:format!("message-{i}") };
            let first = mutate(&db,&request).unwrap();
            assert_eq!(mutate(&db,&request).unwrap(),first);
        })
    }).collect();
    for thread in threads { thread.join().unwrap(); }
    let state = db.forge_get(1).unwrap();
    assert_eq!(state.comment_count,4); assert_eq!(state.revision,5);
    let events = db.forge_timeline(1,0,8).unwrap();
    assert_eq!(events["items"].as_array().unwrap().len(),5);
    for (i,event) in events["items"].as_array().unwrap().iter().enumerate() { assert_eq!(event["revision"],i as u64 + 1); }
}

#[test]
fn error_after_record_write_rolls_back_counter_thread_and_receipt() {
    let (_dir,db,_repo) = database();
    let tx = db.db.begin_write().unwrap();
    tx.open_table(EVENTS).unwrap().insert((1u32,1u64),b"occupied".as_slice()).unwrap();
    tx.commit().unwrap();
    assert!(mutate(&db,&create("initial")).is_err());
    let tx = db.db.begin_read().unwrap();
    assert!(tx.open_table(THREADS).unwrap().get(1).unwrap().is_none());
    assert!(tx.open_table(RECEIPTS).unwrap().get("initial").unwrap().is_none());
    assert!(tx.open_table(META).unwrap().get("forge-last-thread").unwrap().is_none());
    assert_eq!(tx.open_table(EVENTS).unwrap().get((1u32,1u64)).unwrap().unwrap().value(),b"occupied");
}

#[test]
fn retarget_clears_reviews_and_checks_and_stale_reviews_cannot_apply() {
    let (_dir,db,_repo) = database();
    let request = Request::CreatePull { operation_id:"pull".into(),title:"Change".into(),body:"".into(),base:"HEAD".into(),head:"HEAD".into(),required_checks:BTreeMap::new() };
    db.forge_mutate(&request,&digest(&serde_json::to_vec(&request).unwrap()),Some(("1".repeat(40),"2".repeat(40))),None).unwrap();
    mutate(&db,&Request::Review { operation_id:"review".into(),number:1,expected_revision:1,head_oid:"2".repeat(40),decision:ReviewState::Approved }).unwrap();
    let retarget = Request::Retarget { operation_id:"retarget".into(),number:1,expected_revision:2,base:"base".into(),head:"head".into() };
    let updated = db.forge_mutate(&retarget,&digest(&serde_json::to_vec(&retarget).unwrap()),Some(("1".repeat(40),"3".repeat(40))),None).unwrap();
    assert!(updated.pull.unwrap().review.is_none());
    assert!(mutate(&db,&Request::Review { operation_id:"old".into(),number:1,expected_revision:3,head_oid:"2".repeat(40),decision:ReviewState::Approved }).is_err());
    assert_eq!(db.forge_get(1).unwrap().revision,3);
}

#[test]
fn paginated_records_and_events_use_exclusive_numeric_cursors() {
    let (_dir,db,_repo) = database();
    for i in 0..3 { mutate(&db,&create(&format!("create-{i}"))).unwrap(); }
    let first = db.forge_list(0,2).unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(),2);
    assert_eq!(first["next_after"],2);
    let second = db.forge_list(2,2).unwrap();
    assert_eq!(second["items"][0]["number"],3);
    assert!(db.forge_list(u32::MAX,8).unwrap()["items"].as_array().unwrap().is_empty());
    assert!(db.forge_timeline(1,u64::MAX,8).unwrap()["items"].as_array().unwrap().is_empty());
    assert!(Request::List { after:0,limit:9 }.validate().is_err());
}

#[test]
fn forged_identity_extra_fields_and_invalid_domain_values_fail_closed() {
    for request in [
        r#"{"operation":"create_issue","operation_id":"x","title":"x","body":"","actor":"admin"}"#,
        r#"{"operation":"merge","number":1}"#,
        r#"{"operation":"attach_check","operation_id":"x","number":1,"expected_revision":1,"context":"build","job":"job","state":"success"}"#,
    ] { assert!(serde_json::from_str::<Request>(request).is_err()); }
    assert!(Request::CreateIssue { operation_id:"a".into(),title:"\n".into(),body:"".into() }.validate().is_err());
    assert!(Request::Comment { operation_id:"a".into(),number:1,body:" ".into() }.validate().is_err());
    assert!(Request::Get { number:0 }.validate().is_err());
    let (_dir,db,_repo) = database();
    let mut record = mutate(&db,&create("initial")).unwrap();
    record.number = 2;
    let tx = db.db.begin_write().unwrap();
    tx.open_table(THREADS).unwrap().insert(1,bound_bytes(&record).unwrap().as_slice()).unwrap();
    tx.commit().unwrap();
    assert!(db.forge_get(1).is_err());
}

#[test]
fn forge_records_share_action_database_without_changing_existing_receipts() {
    let (_dir,db,repo) = database();
    let module = wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "run") (result i32) i32.const 0))"#).unwrap();
    let module = db.install(&module).unwrap();
    db.submit("job",&module,SnapshotInput { commit:"1".repeat(40),blob:"2".repeat(40),path:"file".into(),bytes:b"input".to_vec() }).unwrap();
    let receipt = db.run_once("job").unwrap();
    db.forge_control(&repo,create("issue")).unwrap();
    assert_eq!(db.get("job").unwrap(),receipt);
    assert_eq!(db.artifact("job").unwrap(),b"");
    let mut first = Vec::new(); db.forge_export(&mut first).unwrap();
    let mut second = Vec::new(); db.forge_export(&mut second).unwrap();
    assert_eq!(first,second);
    let lines: Vec<Value> = first.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect();
    assert_eq!(lines.last().unwrap()["threads"],1);
    assert_eq!(lines.last().unwrap()["events"],1);
    assert_eq!(lines.last().unwrap()["receipts"],1);
    assert_eq!(db.get("job").unwrap(),receipt);
}
