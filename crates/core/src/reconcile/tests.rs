use super::{
    Action, FolderMove, Known, LocalFile, Refusal, RemoteFile, Side, State, Tree, conflict_name, folder_moves, move_state, move_tree, reconcile,
};

fn known(etag: &str, size: u64, mtime: i64) -> Known {
    Known { etag: etag.into(), size, mtime, hash: None }
}

fn local(files: &[(&str, u64, i64)], folders: &[&str]) -> Tree<LocalFile> {
    Tree {
        files: files.iter().map(|(p, size, mtime)| ((*p).to_owned(), LocalFile { size: *size, mtime: *mtime })).collect(),
        folders: folders.iter().map(|f| (*f).to_owned()).collect(),
    }
}

fn remote(files: &[(&str, &str, u64)], folders: &[&str]) -> Tree<RemoteFile> {
    Tree {
        files: files.iter().map(|(p, etag, size)| ((*p).to_owned(), RemoteFile { etag: (*etag).into(), size: *size })).collect(),
        folders: folders.iter().map(|f| (*f).to_owned()).collect(),
    }
}

fn state(files: &[(&str, Known)], folders: &[&str]) -> State {
    State { files: files.iter().map(|(p, k)| ((*p).to_owned(), k.clone())).collect(), folders: folders.iter().map(|f| (*f).to_owned()).collect() }
}

fn plan(s: &State, l: &Tree<LocalFile>, r: &Tree<RemoteFile>) -> Vec<Action> {
    reconcile(s, l, r, false).unwrap()
}

#[test]
fn nothing_changed_nothing_to_do() {
    let s = state(&[("Docs/a.md", known("e1", 3, 100))], &["Docs"]);
    assert_eq!(plan(&s, &local(&[("Docs/a.md", 3, 100)], &["Docs"]), &remote(&[("Docs/a.md", "e1", 3)], &["Docs"])), []);
}

#[test]
fn a_local_edit_uploads_over_the_version_it_started_from() {
    let s = state(&[("a.md", known("e1", 3, 100))], &[]);
    assert_eq!(
        plan(&s, &local(&[("a.md", 5, 200)], &[]), &remote(&[("a.md", "e1", 3)], &[])),
        [Action::UploadChange { path: "a.md".into(), etag: "e1".into() }]
    );
}

#[test]
fn a_cloud_edit_downloads() {
    let s = state(&[("a.md", known("e1", 3, 100))], &[]);
    assert_eq!(plan(&s, &local(&[("a.md", 3, 100)], &[]), &remote(&[("a.md", "e2", 4)], &[])), [Action::Download("a.md".into())]);
}

#[test]
fn edits_on_both_sides_keep_both() {
    let s = state(&[("a.md", known("e1", 3, 100))], &[]);
    assert_eq!(plan(&s, &local(&[("a.md", 5, 200)], &[]), &remote(&[("a.md", "e2", 4)], &[])), [Action::Conflict("a.md".into())]);
}

#[test]
fn new_files_cross_over_with_their_folders() {
    let p = plan(&State::default(), &local(&[("New/Deep/a.md", 1, 1)], &["New", "New/Deep"]), &remote(&[("Cloud/b.md", "e", 2)], &["Cloud"]));
    assert_eq!(
        p,
        [
            Action::MakeRemoteFolder("New".into()),
            Action::MakeRemoteFolder("New/Deep".into()),
            Action::MakeLocalFolder("Cloud".into()),
            Action::Download("Cloud/b.md".into()),
            Action::UploadNew("New/Deep/a.md".into()),
        ]
    );
}

#[test]
fn new_on_both_sides_is_compared_as_a_conflict() {
    assert_eq!(plan(&State::default(), &local(&[("a.md", 1, 1)], &[]), &remote(&[("a.md", "e", 1)], &[])), [Action::Conflict("a.md".into())]);
}

#[test]
fn deletes_follow_unless_the_other_side_changed() {
    let s = state(
        &[
            ("gone-here.md", known("e1", 3, 100)),
            ("gone-there.md", known("e2", 3, 100)),
            ("edited-there.md", known("e3", 3, 100)),
            ("edited-here.md", known("e4", 3, 100)),
        ],
        &[],
    );
    let l = local(&[("gone-there.md", 3, 100), ("edited-here.md", 9, 300)], &[]);
    let r = remote(&[("gone-here.md", "e1", 3), ("edited-there.md", "e3b", 7)], &[]);
    assert_eq!(
        plan(&s, &l, &r),
        [
            // In path order.
            Action::UploadNew("edited-here.md".into()),
            Action::Download("edited-there.md".into()),
            Action::DeleteRemote { path: "gone-here.md".into(), etag: "e1".into() },
            Action::DeleteLocal("gone-there.md".into()),
        ]
    );
}

#[test]
fn gone_on_both_sides_is_forgotten() {
    let s = state(&[("a.md", known("e1", 3, 100))], &[]);
    assert_eq!(plan(&s, &local(&[], &[]), &remote(&[], &[])), [Action::Forget("a.md".into())]);
}

#[test]
fn a_folder_deleted_here_goes_to_the_trash_whole() {
    let s = state(&[("Old/a.md", known("e1", 3, 100)), ("Old/Sub/b.md", known("e2", 4, 100))], &["Old", "Old/Sub", "Old/Sub/Empty"]);
    let r = remote(&[("Old/a.md", "e1", 3), ("Old/Sub/b.md", "e2", 4)], &["Old", "Old/Sub", "Old/Sub/Empty"]);
    let p = plan(&s, &local(&[], &[]), &r);
    assert_eq!(
        p,
        [Action::DeleteRemoteFolder { path: "Old".into(), files: vec![("Old/Sub/b.md".into(), "e2".into()), ("Old/a.md".into(), "e1".into())] }]
    );
}

#[test]
fn a_deleted_folder_with_news_in_the_cloud_goes_file_by_file() {
    let s = state(&[("Old/a.md", known("e1", 3, 100)), ("Old/b.md", known("e2", 4, 100))], &["Old"]);
    // b.md changed in the cloud: it comes back; a.md alone goes.
    let p = plan(&s, &local(&[], &[]), &remote(&[("Old/a.md", "e1", 3), ("Old/b.md", "e9", 5)], &["Old"]));
    assert_eq!(
        p,
        [
            Action::MakeLocalFolder("Old".into()),
            Action::DeleteRemote { path: "Old/a.md".into(), etag: "e1".into() },
            Action::Download("Old/b.md".into())
        ]
    );
    // A new file in the cloud: not deleted unseen.
    let p = plan(&s, &local(&[], &[]), &remote(&[("Old/a.md", "e1", 3), ("Old/b.md", "e2", 4), ("Old/new.md", "e3", 1)], &["Old"]));
    assert!(!p.iter().any(|a| matches!(a, Action::DeleteRemoteFolder { .. })), "{p:?}");
}

#[test]
fn a_folder_deleted_on_one_side_goes_once_empty() {
    let s = state(&[("Old/a.md", known("e1", 3, 100))], &["Old", "Old/Sub"]);
    // Deleted in the cloud (folder and file); this computer still has both.
    let p = plan(&s, &local(&[("Old/a.md", 3, 100)], &["Old", "Old/Sub"]), &remote(&[], &[]));
    assert_eq!(p, [Action::DeleteLocal("Old/a.md".into()), Action::RemoveLocalFolder("Old/Sub".into()), Action::RemoveLocalFolder("Old".into()),]);
}

#[test]
fn a_folder_with_new_files_is_not_removed() {
    let s = state(&[], &["Old"]);
    // Deleted in the cloud, but someone saved a new file into it locally.
    let p = plan(&s, &local(&[("Old/new.md", 1, 1)], &["Old"]), &remote(&[], &[]));
    assert_eq!(p, [Action::MakeRemoteFolder("Old".into()), Action::UploadNew("Old/new.md".into())]);
}

#[test]
fn mass_deletes_pause_until_confirmed() {
    let files: Vec<(String, Known)> = (0..20).map(|i| (format!("f{i}.md"), known("e", 1, 1))).collect();
    let s = State { files: files.into_iter().collect(), folders: Default::default() };
    let r = remote(&s.files.keys().map(|p| (p.as_str(), "e", 1)).collect::<Vec<_>>(), &[]);
    // The local folder looks empty: every file would be deleted in the cloud.
    assert_eq!(reconcile(&s, &local(&[], &[]), &r, false), Err(Refusal::TooManyDeletes { deletes: 20, limit: 10 }));
    assert_eq!(reconcile(&s, &local(&[], &[]), &r, true).unwrap().len(), 20);
}

#[test]
fn conflict_copies_keep_the_extension() {
    assert_eq!(conflict_name("Docs/Q3 report.docx", "2026-11-13 14.32", "Studio"), "Docs/Q3 report (conflict 2026-11-13 14.32 Studio).docx");
    assert_eq!(conflict_name("README", "2026-11-13 14.32", "Laptop"), "README (conflict 2026-11-13 14.32 Laptop)");
    assert_eq!(conflict_name(".env", "t", "d"), ".env (conflict t d)");
}

#[test]
fn conflict_copies_lead_back_to_their_original() {
    use super::original_of_conflict;
    for original in ["Docs/Q3 report.docx", "README", "a.tar.gz"] {
        let copy = conflict_name(original, "2026-11-13 14.32", "Studio");
        assert_eq!(original_of_conflict(&copy).as_deref(), Some(original), "{copy}");
    }
    assert_eq!(original_of_conflict("Docs/plain.docx"), None);
}

#[test]
fn a_local_rename_is_one_move_in_the_cloud() {
    let s = state(&[("c1.txt", known("e1", 3, 100))], &[]);
    let p = plan(&s, &local(&[("c1-renamed.txt", 3, 100)], &[]), &remote(&[("c1.txt", "e1", 3)], &[]));
    assert_eq!(p, [Action::MoveRemote { from: "c1.txt".into(), to: "c1-renamed.txt".into(), etag: "e1".into() }]);
}

#[test]
fn a_move_into_a_new_folder_makes_the_folder_then_moves() {
    let s = state(&[("a.txt", known("e1", 3, 100))], &[]);
    let p = plan(&s, &local(&[("sub/deep/a.txt", 3, 100)], &["sub", "sub/deep"]), &remote(&[("a.txt", "e1", 3)], &[]));
    assert_eq!(
        p,
        [
            Action::MakeRemoteFolder("sub".into()),
            Action::MakeRemoteFolder("sub/deep".into()),
            Action::MoveRemote { from: "a.txt".into(), to: "sub/deep/a.txt".into(), etag: "e1".into() },
        ]
    );
}

#[test]
fn a_cloud_rename_renames_the_local_file() {
    let s = state(&[("Docs/a.md", known("\"h1-100\"", 3, 100))], &["Docs"]);
    let p = plan(&s, &local(&[("Docs/a.md", 3, 100)], &["Docs"]), &remote(&[("Docs/b.md", "h1-100", 3)], &["Docs"]));
    assert_eq!(p, [Action::MoveLocal { from: "Docs/a.md".into(), to: "Docs/b.md".into() }]);
}

#[test]
fn a_renamed_folder_is_moves_not_a_mass_delete() {
    let files: Vec<(String, Known)> = (0..20).map(|i| (format!("Old/{i}.txt"), known(&format!("e{i}"), 10 + i, 100 + i as i64))).collect();
    let s = State { files: files.iter().cloned().collect(), folders: ["Old".to_owned()].into() };
    let l = Tree {
        files: (0..20).map(|i| (format!("New/{i}.txt"), LocalFile { size: 10 + i, mtime: 100 + i as i64 })).collect(),
        folders: ["New".to_owned()].into(),
    };
    let r = Tree {
        files: (0..20).map(|i| (format!("Old/{i}.txt"), RemoteFile { etag: format!("e{i}"), size: 10 + i })).collect(),
        folders: ["Old".to_owned()].into(),
    };
    let p = reconcile(&s, &l, &r, false).expect("moves are not deletes");
    assert_eq!(p.iter().filter(|a| matches!(a, Action::MoveRemote { .. })).count(), 20);
    assert_eq!(p.first(), Some(&Action::MakeRemoteFolder("New".into())));
    assert_eq!(p.last(), Some(&Action::RemoveRemoteFolder("Old".into())));
}

#[test]
fn ambiguous_moves_stay_upload_and_delete() {
    // Two new files look like the one that went: no guess.
    let s = state(&[("a.txt", known("e1", 3, 100))], &[]);
    let p = plan(&s, &local(&[("b.txt", 3, 100), ("c.txt", 3, 100)], &[]), &remote(&[("a.txt", "e1", 3)], &[]));
    assert!(p.contains(&Action::DeleteRemote { path: "a.txt".into(), etag: "e1".into() }), "{p:?}");
    assert!(p.contains(&Action::UploadNew("b.txt".into())) && p.contains(&Action::UploadNew("c.txt".into())));
    // Another time or size is another file.
    let p = plan(&s, &local(&[("b.txt", 3, 101)], &[]), &remote(&[("a.txt", "e1", 3)], &[]));
    assert!(p.contains(&Action::UploadNew("b.txt".into())), "{p:?}");
}

fn fm(from: &str, to: &str, side: Side) -> FolderMove {
    FolderMove { from: from.into(), to: to.into(), side }
}

/// `Old` holds `a.txt` and `Sub/b.txt`, synced.
fn synced_old() -> State {
    state(&[("Old/a.txt", known("\"h1-1\"", 3, 100)), ("Old/Sub/b.txt", known("\"h2-1\"", 4, 200))], &["Old", "Old/Sub"])
}

#[test]
fn a_folder_renamed_here_is_one_folder_move() {
    let s = synced_old();
    let l = local(&[("New/a.txt", 3, 100), ("New/Sub/b.txt", 4, 200)], &["New", "New/Sub"]);
    let r = remote(&[("Old/a.txt", "\"h1-1\"", 3), ("Old/Sub/b.txt", "\"h2-1\"", 4)], &["Old", "Old/Sub"]);
    assert_eq!(folder_moves(&s, &l, &r), [fm("Old", "New", Side::Local)], "only the top-most folder");

    // Once it is moved in the cloud too, there is nothing left to do.
    let (mut s, mut r) = (s, r);
    move_state(&mut s, "Old", "New");
    move_tree(&mut r, "Old", "New");
    assert_eq!(s.folders.iter().map(String::as_str).collect::<Vec<_>>(), ["New", "New/Sub"]);
    assert_eq!(plan(&s, &l, &r), []);
}

#[test]
fn a_folder_moved_in_the_cloud_is_one_local_rename() {
    let s = synced_old();
    let l = local(&[("Old/a.txt", 3, 100), ("Old/Sub/b.txt", 4, 200)], &["Old", "Old/Sub"]);
    let r = remote(&[("Archive/New/a.txt", "h1-1", 3), ("Archive/New/Sub/b.txt", "h2-1", 4)], &["Archive", "Archive/New", "Archive/New/Sub"]);
    assert_eq!(folder_moves(&s, &l, &r), [fm("Old", "Archive/New", Side::Remote)]);
}

#[test]
fn a_folder_move_carries_changes_made_on_the_other_side() {
    // Renamed here while the cloud copy of a.txt was edited elsewhere: the folder still moves (so
    // there is one folder afterwards), and the edit comes down into the new place.
    let s = synced_old();
    let l = local(&[("New/a.txt", 3, 100), ("New/Sub/b.txt", 4, 200), ("New/extra.txt", 1, 300)], &["New", "New/Sub"]);
    let r = remote(&[("Old/a.txt", "\"h9-2\"", 5), ("Old/Sub/b.txt", "\"h2-1\"", 4)], &["Old", "Old/Sub"]);
    assert_eq!(folder_moves(&s, &l, &r), [fm("Old", "New", Side::Local)]);
    let (mut s, mut r) = (s, r);
    move_state(&mut s, "Old", "New");
    move_tree(&mut r, "Old", "New");
    assert_eq!(plan(&s, &l, &r), [Action::Download("New/a.txt".into()), Action::UploadNew("New/extra.txt".into())]);
}

#[test]
fn partial_or_ambiguous_folder_matches_are_handled_file_by_file() {
    let s = synced_old();
    let r = remote(&[("Old/a.txt", "\"h1-1\"", 3), ("Old/Sub/b.txt", "\"h2-1\"", 4)], &["Old", "Old/Sub"]);
    // A file edited in the new place: not the same folder for sure.
    let l = local(&[("New/a.txt", 9, 999), ("New/Sub/b.txt", 4, 200)], &["New", "New/Sub"]);
    assert!(folder_moves(&s, &l, &r).iter().all(|m| m.from != "Old"), "{:?}", folder_moves(&s, &l, &r));
    // A file missing in the new place.
    let l = local(&[("New/a.txt", 3, 100)], &["New"]);
    assert_eq!(folder_moves(&s, &l, &r), []);
    // Two copies: no guess.
    let l = local(&[("A/a.txt", 3, 100), ("A/Sub/b.txt", 4, 200), ("B/a.txt", 3, 100), ("B/Sub/b.txt", 4, 200)], &["A", "A/Sub", "B", "B/Sub"]);
    assert_eq!(folder_moves(&s, &l, &r), []);
    // Still there on both sides: nothing moved.
    let l = local(
        &[("Old/a.txt", 3, 100), ("Old/Sub/b.txt", 4, 200), ("New/a.txt", 3, 100), ("New/Sub/b.txt", 4, 200)],
        &["Old", "Old/Sub", "New", "New/Sub"],
    );
    assert_eq!(folder_moves(&s, &l, &r), []);
}

#[test]
fn a_sub_folder_moved_out_of_its_folder_moves_alone() {
    let s = synced_old();
    let l = local(&[("Old/a.txt", 3, 100), ("Moved/b.txt", 4, 200)], &["Old", "Moved"]);
    let r = remote(&[("Old/a.txt", "\"h1-1\"", 3), ("Old/Sub/b.txt", "\"h2-1\"", 4)], &["Old", "Old/Sub"]);
    assert_eq!(folder_moves(&s, &l, &r), [fm("Old/Sub", "Moved", Side::Local)]);
}

#[test]
fn a_file_and_a_folder_with_one_name_set_the_local_one_aside() {
    // Here a folder `t5/clash` with a file in it; in the cloud a file `t5/clash`.
    let l = local(&[("t5/clash/inner.txt", 1, 1)], &["t5", "t5/clash"]);
    let r = remote(&[("t5/clash", "e", 2)], &["t5"]);
    let s = state(&[], &["t5"]);
    assert_eq!(plan(&s, &l, &r), [Action::LocalClash("t5/clash".into())]);
    // And the other way round.
    let l = local(&[("t5/clash", 2, 1)], &["t5"]);
    let r = remote(&[("t5/clash/inner.txt", "e", 1)], &["t5", "t5/clash"]);
    assert_eq!(plan(&s, &l, &r), [Action::LocalClash("t5/clash".into())]);
}

/// T-052: the synced folder `t17/sw` (with `in.txt`) and the synced file `t17/sf`.
fn replaced_before() -> State {
    state(&[("t17/sw/in.txt", known("e1", 2, 100)), ("t17/sf", known("e2", 3, 100))], &["t17", "t17/sw"])
}

#[test]
fn a_synced_folder_replaced_by_a_file_here_replaces_it_in_the_cloud() {
    let s = replaced_before();
    let l = local(&[("t17/sw", 2, 500), ("t17/sf", 3, 100)], &["t17"]);
    let r = remote(&[("t17/sw/in.txt", "e1", 2), ("t17/sf", "e2", 3)], &["t17", "t17/sw"]);
    // The folder goes to the trash as one item first, then the file goes up.
    assert_eq!(
        plan(&s, &l, &r),
        [
            Action::DeleteRemoteFolder { path: "t17/sw".into(), files: vec![("t17/sw/in.txt".into(), "e1".into())] },
            Action::UploadNew("t17/sw".into())
        ]
    );
    // An empty synced folder: removed, then the file goes up.
    let s = state(&[], &["t17", "t17/sw"]);
    let p = plan(&s, &local(&[("t17/sw", 2, 500)], &["t17"]), &remote(&[], &["t17", "t17/sw"]));
    assert_eq!(p, [Action::RemoveRemoteFolder("t17/sw".into()), Action::UploadNew("t17/sw".into())]);
}

#[test]
fn a_synced_file_replaced_by_a_folder_here_replaces_it_in_the_cloud() {
    let s = replaced_before();
    let l = local(&[("t17/sw/in.txt", 2, 100), ("t17/sf/a.txt", 1, 500), ("t17/sf/Deep/b.txt", 1, 500)], &["t17", "t17/sw", "t17/sf", "t17/sf/Deep"]);
    let r = remote(&[("t17/sw/in.txt", "e1", 2), ("t17/sf", "e2", 3)], &["t17", "t17/sw"]);
    assert_eq!(
        plan(&s, &l, &r),
        [
            Action::DeleteRemote { path: "t17/sf".into(), etag: "e2".into() },
            Action::MakeRemoteFolder("t17/sf".into()),
            Action::MakeRemoteFolder("t17/sf/Deep".into()),
            Action::UploadNew("t17/sf/Deep/b.txt".into()),
            Action::UploadNew("t17/sf/a.txt".into()),
        ]
    );
}

#[test]
fn a_synced_folder_replaced_by_a_file_in_the_cloud_is_replaced_here() {
    let s = replaced_before();
    let l = local(&[("t17/sw/in.txt", 2, 100), ("t17/sf", 3, 100)], &["t17", "t17/sw"]);
    let r = remote(&[("t17/sw", "e9", 2), ("t17/sf", "e2", 3)], &["t17"]);
    assert_eq!(
        plan(&s, &l, &r),
        [Action::DeleteLocal("t17/sw/in.txt".into()), Action::RemoveLocalFolder("t17/sw".into()), Action::Download("t17/sw".into())]
    );
}

#[test]
fn a_synced_file_replaced_by_a_folder_in_the_cloud_is_replaced_here() {
    let s = replaced_before();
    let l = local(&[("t17/sw/in.txt", 2, 100), ("t17/sf", 3, 100)], &["t17", "t17/sw"]);
    let r = remote(&[("t17/sw/in.txt", "e1", 2), ("t17/sf/a.txt", "e8", 1)], &["t17", "t17/sw", "t17/sf"]);
    assert_eq!(
        plan(&s, &l, &r),
        [Action::DeleteLocal("t17/sf".into()), Action::MakeLocalFolder("t17/sf".into()), Action::Download("t17/sf/a.txt".into())]
    );
}

#[test]
fn a_replacement_on_one_side_and_a_change_on_the_other_is_a_conflict() {
    let s = replaced_before();
    // Folder → file here, while the cloud folder got a new file.
    let l = local(&[("t17/sw", 2, 500), ("t17/sf", 3, 100)], &["t17"]);
    let r = remote(&[("t17/sw/in.txt", "e1", 2), ("t17/sw/new.txt", "e5", 1), ("t17/sf", "e2", 3)], &["t17", "t17/sw"]);
    assert_eq!(plan(&s, &l, &r), [Action::LocalClash("t17/sw".into())]);
    // Folder → file here, while the cloud file in it changed.
    let r = remote(&[("t17/sw/in.txt", "e7", 4), ("t17/sf", "e2", 3)], &["t17", "t17/sw"]);
    assert_eq!(plan(&s, &l, &r), [Action::LocalClash("t17/sw".into())]);
    // File → folder here, while the cloud file changed.
    let l = local(&[("t17/sw/in.txt", 2, 100), ("t17/sf/a.txt", 1, 500)], &["t17", "t17/sw", "t17/sf"]);
    let r = remote(&[("t17/sw/in.txt", "e1", 2), ("t17/sf", "e6", 9)], &["t17", "t17/sw"]);
    assert_eq!(plan(&s, &l, &r), [Action::LocalClash("t17/sf".into())]);
    // Folder → file in the cloud, while a file in the local folder changed or a new one was saved.
    let r = remote(&[("t17/sw", "e9", 2), ("t17/sf", "e2", 3)], &["t17"]);
    for l in [
        local(&[("t17/sw/in.txt", 5, 600), ("t17/sf", 3, 100)], &["t17", "t17/sw"]),
        local(&[("t17/sw/in.txt", 2, 100), ("t17/sw/new.txt", 1, 600), ("t17/sf", 3, 100)], &["t17", "t17/sw"]),
    ] {
        assert_eq!(plan(&s, &l, &r), [Action::LocalClash("t17/sw".into())]);
    }
    // File → folder in the cloud, while the local file changed.
    let l = local(&[("t17/sw/in.txt", 2, 100), ("t17/sf", 8, 600)], &["t17", "t17/sw"]);
    let r = remote(&[("t17/sw/in.txt", "e1", 2), ("t17/sf/a.txt", "e8", 1)], &["t17", "t17/sw", "t17/sf"]);
    assert_eq!(plan(&s, &l, &r), [Action::LocalClash("t17/sf".into())]);
}

#[test]
fn a_second_conflict_copy_counts_instead_of_nesting() {
    use super::{free_conflict_name, original_of_conflict};
    let first = conflict_name("x.txt", "2026-11-14 01.30", "Studio");
    assert_eq!(first, "x (conflict 2026-11-14 01.30 Studio).txt");
    let second = free_conflict_name("x.txt", "2026-11-14 01.30", "Studio", |n| n == first);
    assert_eq!(second, "x (conflict 2026-11-14 01.30 Studio 2).txt");
    let third = free_conflict_name("x.txt", "2026-11-14 01.30", "Studio", |n| n == first || n == second);
    assert_eq!(third, "x (conflict 2026-11-14 01.30 Studio 3).txt");
    // A conflict copy's own conflict is named after the original.
    assert_eq!(conflict_name(&first, "2026-11-14 02.00", "Laptop"), "x (conflict 2026-11-14 02.00 Laptop).txt");
    assert_eq!(original_of_conflict(&second).as_deref(), Some("x.txt"));
}

#[test]
fn an_edit_follows_a_rename_made_elsewhere() {
    // Renamed in the cloud while edited here: renamed here too (the edit goes up next cycle).
    let s = state(&[("f.txt", known("\"h1-1\"", 3, 100))], &[]);
    let p = plan(&s, &local(&[("f.txt", 9, 200)], &[]), &remote(&[("renamed-A.txt", "\"h1-1\"", 3)], &[]));
    assert_eq!(p, [Action::MoveLocal { from: "f.txt".into(), to: "renamed-A.txt".into() }]);
    // Renamed here while edited in the cloud: moved there too (the edit comes down next cycle).
    let p = plan(&s, &local(&[("renamed-B.txt", 3, 100)], &[]), &remote(&[("f.txt", "\"h9-2\"", 7)], &[]));
    assert_eq!(p, [Action::MoveRemote { from: "f.txt".into(), to: "renamed-B.txt".into(), etag: "\"h9-2\"".into() }]);
}

/// Found by the property tests (`properties.rs`): one changed local file was taken both as where a
/// file moved to here and as where a file was renamed from in the cloud, so the plan held two moves
/// through the same path. Carried out in path order, either the second move failed and the next
/// cycle uploaded over the renamed cloud file (its bytes kept only as a version), or both ran and
/// the two sides kept different bytes under one name, recorded as synced. Now the move here wins
/// (the file here is the one moved in, not an edit), and the renamed cloud file comes down as new.
#[test]
fn one_file_is_never_two_moves() {
    // Here: `a.txt` moved onto `B/a.txt`, replacing it. In the cloud: `B/a.txt` renamed to `b.md`.
    let s = state(&[("a.txt", known("e0", 5, 100)), ("B/a.txt", known("e3", 7, 200))], &["B"]);
    let l = local(&[("B/a.txt", 5, 100)], &["B"]);
    let r = remote(&[("a.txt", "e0", 5), ("b.md", "e3", 7)], &["B"]);
    assert_eq!(
        plan(&s, &l, &r),
        [Action::MoveRemote { from: "a.txt".into(), to: "B/a.txt".into(), etag: "e0".into() }, Action::Download("b.md".into())]
    );
}

/// The other way round: a file moved in the cloud onto one renamed here. The cloud file at the old
/// name is the moved one (its ETag), not an edit of the renamed one, so it isn't moved to the new
/// name (where the next pass would download it over the renamed file): both are kept.
#[test]
fn a_file_moved_onto_in_the_cloud_is_not_followed_to_a_rename_here() {
    // In the cloud: `a.txt` moved onto `B/a.txt`, replacing it. Here: `B/a.txt` renamed to `b.md`.
    let s = state(&[("a.txt", known("e0", 5, 100)), ("B/a.txt", known("e3", 7, 200))], &["B"]);
    let l = local(&[("a.txt", 5, 100), ("b.md", 7, 200)], &["B"]);
    let r = remote(&[("B/a.txt", "e0", 5)], &["B"]);
    assert_eq!(plan(&s, &l, &r), [Action::Download("B/a.txt".into()), Action::DeleteLocal("a.txt".into()), Action::UploadNew("b.md".into())]);
}

/// Found by the property tests (`properties.rs`), not fixed yet: when a synced folder is replaced
/// by a file here, everything touching it is put first, in the order removals, folders to make,
/// the rest. A file moved out of that folder then comes after the folder's removal (which finds it
/// not empty and does nothing) and before the folder it moves into is made (so the move fails).
/// Nothing is lost, but it takes extra passes, with errors in the activity list on the way.
#[test]
#[ignore = "known bug: a replaced folder is removed before files are moved out of it (fix in its own pull request)"]
fn a_replaced_folder_is_emptied_before_it_goes() {
    // Here: `A/S/S` moved to the new folder `B`, then a file saved as `A/S` where the folder was.
    let s = state(&[("A/S/S", known("e0", 3, 100))], &["A", "A/S"]);
    let l = local(&[("A/S", 5, 200), ("B/a.txt", 3, 100)], &["A", "B"]);
    let r = remote(&[("A/S/S", "e0", 3)], &["A", "A/S"]);
    let p = plan(&s, &l, &r);
    let at = |want: &Action| p.iter().position(|a| a == want).unwrap_or(usize::MAX);
    let moved = at(&Action::MoveRemote { from: "A/S/S".into(), to: "B/a.txt".into(), etag: "e0".into() });
    assert!(at(&Action::MakeRemoteFolder("B".into())) < moved, "{p:?}");
    assert!(moved < at(&Action::RemoveRemoteFolder("A/S".into())), "{p:?}");
}
