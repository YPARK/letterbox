use super::*;
use crate::trash::{self, Entry};

/// The UIDs of `folder`, newest first.
fn uids(store: &mut Demo, folder: &str) -> Vec<u32> {
    store
        .page(folder, 0, 100)
        .unwrap()
        .summaries
        .iter()
        .map(|s| s.uid)
        .collect()
}

/// Move the newest of `from` to Trash, recorded as trashed at `at`.
fn trash_one(store: &mut Demo, ledger: &mut trash::Ledger, from: &str, at: i64) -> Entry {
    let uid = uids(store, from)[0];
    let moved = store.relocate(from, uid, "Trash", "").unwrap();
    let entry = Entry {
        account: "you@example.com".into(),
        message_id: String::new(),
        folder: "Trash".into(),
        uid: moved,
        origin: from.into(),
        at,
    };
    ledger.record(entry.clone()).unwrap();
    entry
}

#[test]
fn the_ledger_says_what_was_trashed_between_two_times() {
    let mut ledger = scratch_ledger();
    let entry = |uid, account: &str, at| Entry {
        account: account.into(),
        message_id: format!("<{uid}@x>"),
        folder: "Trash".into(),
        uid,
        origin: "INBOX".into(),
        at,
    };
    ledger.record(entry(1, "a@b.c", 100)).unwrap();
    ledger.record(entry(2, "a@b.c", 200)).unwrap();
    ledger.record(entry(3, "d@e.f", 200)).unwrap();
    ledger.record(entry(4, "a@b.c", 300)).unwrap();
    let uids = |v: Vec<Entry>| v.into_iter().map(|e| e.uid).collect::<Vec<_>>();
    assert_eq!(uids(ledger.between(150, 250, None)), [2, 3]);
    assert_eq!(uids(ledger.between(150, 300, Some("a@b.c"))), [2, 4]);
    assert!(ledger.between(400, 500, None).is_empty());
}

#[test]
fn what_was_trashed_goes_back_where_it_was_and_is_forgotten() {
    let mut store = Demo::new();
    let mut ledger = scratch_ledger();
    let inbox = uids(&mut store, "INBOX").len();
    let sent = uids(&mut store, "Sent").len();
    let a = trash_one(&mut store, &mut ledger, "INBOX", 1000);
    let b = trash_one(&mut store, &mut ledger, "Sent", 1001);
    // Trashed earlier, on purpose: left where it is.
    let kept = trash_one(&mut store, &mut ledger, "INBOX", 10);
    assert_eq!(uids(&mut store, "INBOX").len(), inbox - 2);

    let entries = ledger.between(900, 1100, None);
    let done = trash::restore(&mut ledger, &entries, &mut store);
    assert!(done.iter().all(|r| r.result.is_ok()), "{done:?}");
    assert_eq!(done.len(), 2);
    assert_eq!(uids(&mut store, "INBOX").len(), inbox - 1);
    assert_eq!(uids(&mut store, "Sent").len(), sent);
    assert_eq!(uids(&mut store, "Trash"), [kept.uid]);
    // Restored, they are no longer the ledger's to remove after 7 days.
    assert_eq!(ledger.between(0, i64::MAX, None), [kept]);
    assert!(!ledger.between(0, i64::MAX, None).contains(&a));
    assert!(!ledger.between(0, i64::MAX, None).contains(&b));
}

#[test]
fn a_message_that_cannot_be_put_back_is_kept_in_the_ledger_and_said() {
    let mut store = Demo::new();
    let mut ledger = scratch_ledger();
    let a = trash_one(&mut store, &mut ledger, "INBOX", 1000);
    let b = trash_one(&mut store, &mut ledger, "Archive", 1001);
    // Already taken out of Trash by hand.
    store.relocate("Trash", a.uid, "INBOX", "").unwrap();

    let entries = ledger.between(0, i64::MAX, None);
    let done = trash::restore(&mut ledger, &entries, &mut store);
    let failed: Vec<&Entry> = done
        .iter()
        .filter(|r| r.result.is_err())
        .map(|r| &r.entry)
        .collect();
    assert_eq!(failed, [&a]);
    assert_eq!(ledger.between(0, i64::MAX, None), [a]);
    assert!(!uids(&mut store, "Archive").is_empty());
    assert!(!uids(&mut store, "Trash").contains(&b.uid));
}

/// A store that moves as asked, and then cannot tell where to: as a server
/// that sends no new UID, and the Message-ID not found after.
struct Untold(Demo);

impl Store for Untold {
    fn folders(&mut self) -> anyhow::Result<Vec<Folder>> {
        self.0.folders()
    }
    fn counts(&mut self, folder: &str) -> anyhow::Result<(u32, u32)> {
        self.0.counts(folder)
    }
    fn page(
        &mut self,
        folder: &str,
        offset: usize,
        n: usize,
    ) -> anyhow::Result<crate::store::Page> {
        self.0.page(folder, offset, n)
    }
    fn raw(&mut self, folder: &str, uid: u32) -> anyhow::Result<Vec<u8>> {
        self.0.raw(folder, uid)
    }
    fn set(&mut self, folder: &str, uid: u32, flag: Flag, on: bool) -> anyhow::Result<()> {
        self.0.set(folder, uid, flag, on)
    }
    fn search(
        &mut self,
        folder: &str,
        words: &str,
        n: usize,
    ) -> anyhow::Result<Vec<mail::Summary>> {
        self.0.search(folder, words, n)
    }
    fn relocate(
        &mut self,
        from: &str,
        uid: u32,
        to: &str,
        message_id: &str,
    ) -> anyhow::Result<u32> {
        self.0.relocate(from, uid, to, message_id)?;
        anyhow::bail!("it was moved, but its new place could not be told")
    }
    fn append(&mut self, folder: &str, raw: &[u8], seen: bool) -> anyhow::Result<()> {
        self.0.append(folder, raw, seen)
    }
    fn holds(&mut self, folder: &str, message_id: &str) -> anyhow::Result<bool> {
        self.0.holds(folder, message_id)
    }
    fn create(&mut self, path: &str) -> anyhow::Result<()> {
        self.0.create(path)
    }
    fn address(&self) -> &str {
        self.0.address()
    }
    fn send(&mut self, raw: &[u8]) -> anyhow::Result<String> {
        self.0.send(raw)
    }
}

#[test]
fn one_moved_whose_new_place_is_not_told_is_put_back_once_its_folder_has_it() {
    let mut store = Demo::new();
    let mut ledger = scratch_ledger();
    let mut a = trash_one(&mut store, &mut ledger, "INBOX", 1000);
    // Known by its Message-ID, as the servers' messages are.
    let raw = store.raw("Trash", a.uid).unwrap();
    a.message_id = crate::mail::field(&raw, "Message-ID").unwrap();
    ledger.forget(&a.account, &a.folder, a.uid).unwrap();
    ledger.record(a.clone()).unwrap();
    let mut untold = Untold(store.handle());
    let done = trash::restore(&mut ledger, std::slice::from_ref(&a), &mut untold);
    assert!(done[0].result.is_ok(), "{done:?}");
    assert!(ledger.between(0, i64::MAX, None).is_empty());
    assert!(!uids(&mut store, "Trash").contains(&a.uid));
}

#[test]
fn one_gone_from_trash_and_not_back_where_it_was_is_not_counted_put_back() {
    let mut store = Demo::new();
    let mut ledger = scratch_ledger();
    let mut a = trash_one(&mut store, &mut ledger, "INBOX", 1000);
    let raw = store.raw("Trash", a.uid).unwrap();
    a.message_id = crate::mail::field(&raw, "Message-ID").unwrap();
    ledger.forget(&a.account, &a.folder, a.uid).unwrap();
    ledger.record(a.clone()).unwrap();
    // Out of Trash, but not back in the Inbox: as if removed for good.
    store.relocate("Trash", a.uid, "Junk", "").unwrap();
    let done = trash::restore(&mut ledger, std::slice::from_ref(&a), &mut store);
    assert!(done[0].result.is_err(), "{done:?}");
    assert_eq!(ledger.between(0, i64::MAX, None), [a], "kept, to say");
}

#[test]
fn what_restores_is_written_once_however_many() {
    let mut store = Demo::new();
    let mut ledger = scratch_ledger();
    let entries: Vec<Entry> = (0..3)
        .map(|k| trash_one(&mut store, &mut ledger, "INBOX", 1000 + k))
        .collect();
    let done = trash::restore(&mut ledger, &entries, &mut store);
    assert!(done.iter().all(|r| r.result.is_ok()), "{done:?}");
    assert!(ledger.between(0, i64::MAX, None).is_empty());
    assert_eq!(ledger.saves(), 4, "three recorded, and one forgetting all");
}

#[test]
fn the_index_is_held_by_one_letterbox_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let held = crate::index::lock(dir.path()).unwrap();
    let err = crate::index::lock(dir.path()).unwrap_err();
    assert!(err.to_string().contains("letterbox is open"), "{err}");
    drop(held);
    assert!(crate::index::lock(dir.path()).is_ok());
}

#[test]
fn a_time_is_read_only_as_one_and_never_a_date_typed_without_dashes() {
    // Typed without dashes, a date is no number of seconds: refused.
    let err = crate::when("20261027").unwrap_err();
    assert!(err.to_string().contains("2026-10-27"), "{err}");
    assert!(crate::when("13:50").is_err());
    // Seconds since 1970, as long as they are now.
    assert_eq!(crate::when("1793041545").unwrap(), 1793041545);
    assert!(crate::when("2026-10-27 13:50").is_ok());
    assert!(crate::when("2026-10-27T13:50:00-07:00").is_ok());
}

#[test]
fn what_would_be_forgotten_is_counted_before_it_is() {
    let mut index = crate::index::Index::memory();
    let mut store = Demo::new();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let shared = std::sync::Mutex::new(index);
    let now = jiff::Timestamp::now().as_second();
    crate::index::sync::pass(&mut store, &shared, now, &stop, &mut |_| {}).unwrap();
    index = shared.into_inner().unwrap();
    let keys: Vec<_> = index.records.keys().take(3).cloned().collect();
    for k in &keys {
        index.teach(k, true).unwrap();
    }
    let n = index.filter.taught.len();
    assert_eq!(index.unteach_count(0, i64::MAX), 3);
    assert_eq!(index.filter.taught.len(), n, "counted, nothing forgotten");
}
