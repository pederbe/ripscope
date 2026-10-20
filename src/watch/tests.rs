use super::*;

#[test]
fn stored_names_have_their_directories() {
    assert_eq!(parent_of(b"a.c"), b"");
    assert_eq!(parent_of(b"src/a.c"), b"src");
    assert_eq!(parent_of(b"../include/a.h"), b"../include");
    assert_eq!(parent_of(b"/a.h"), b"/");
    assert_eq!(parent_of(b"/usr/include/a.h"), b"/usr/include");
}

#[cfg(unix)]
#[test]
fn paths_below_the_root_are_known_relative_to_it() {
    let root = Path::new("/work/project");
    assert_eq!(key(root, Path::new("/work/project/src/a.c")), b"src/a.c");
    assert_eq!(key(root, Path::new("/work/project/../h/a.h")), b"../h/a.h");
    assert_eq!(
        key(root, Path::new("/work/projects/a.c")),
        b"/work/projects/a.c"
    );
    assert_eq!(
        key(root, Path::new("/usr/include/a.h")),
        b"/usr/include/a.h"
    );
}

#[cfg(windows)]
#[test]
fn paths_below_the_root_are_known_relative_to_it_without_case() {
    let root = Path::new(r"C:\Work\Project");
    assert_eq!(key(root, Path::new(r"C:\Work\Project\Src\A.c")), b"src/a.c");
    assert_eq!(key(root, Path::new(r"c:\work\project\src/a.c")), b"src/a.c");
    assert_eq!(
        key(root, Path::new(r"C:\Work\Projects\a.c")),
        b"c:/work/projects/a.c"
    );
    assert_eq!(
        key(root, Path::new(r"\\?\C:\Work\Project\Src\a.h")),
        b"src/a.h"
    );
    let share = Path::new(r"\\server\share\project");
    assert_eq!(
        key(share, Path::new(r"\\?\UNC\server\share\project\a.h")),
        b"a.h"
    );
}

fn covered(discovery: Option<bool>) -> Covered {
    let root = if cfg!(windows) {
        PathBuf::from(r"C:\work")
    } else {
        PathBuf::from("/work")
    };
    let rule = Rule {
        root,
        discovery,
        list: None,
    };
    Covered::with(rule, None)
}

#[test]
fn discovery_looks_where_its_rule_recurses() {
    let recursive = covered(Some(true));
    assert!(recursive.discovers(b"a.c"));
    assert!(recursive.discovers(b"src/a.c"));
    assert!(!recursive.discovers(b"../a.c"));
    assert!(!recursive.discovers(b"/usr/include/a.h"));
    let flat = covered(Some(false));
    assert!(flat.discovers(b"a.c"));
    assert!(!flat.discovers(b"src/a.c"));
    assert!(!covered(None).discovers(b"a.c"));
}

#[test]
fn editor_files_and_the_index_change_nothing_a_refresh_finds() {
    let mut watch = covered(Some(true));
    watch.names.insert(b"src/a.c".to_vec());
    watch.directories.insert(b"src".to_vec());
    let index = Watch::new(Path::new("ripscope.db")).index;
    let root = watch.rule.root.clone();
    let changed = |watch: &Covered, name: &str, directory: Option<bool>| {
        watch.relevant(&index, &root.join(name), Kind::Changed, directory)
    };
    assert!(changed(&watch, "src/a.c", Some(false)));
    assert!(changed(&watch, "src/b.h", Some(false)));
    // Linux reports a directory's own metadata only when it is changed itself.
    assert_eq!(changed(&watch, "src", None), !cfg!(windows));
    assert!(watch.relevant(&index, &root.join("src"), Kind::Removed, None));
    assert!(changed(&watch, "lib", Some(true)));
    for noise in [
        "src/.a.c.swp",
        "src/a.c~",
        "src/4913",
        "src/#a.c#",
        "src/a.o",
        "ripscope.db",
        "ripscope.db-wal",
        "ripscope.db-shm",
        ".ripscope-1-1.sync",
    ] {
        assert!(!changed(&watch, noise, Some(false)), "{noise}");
    }
    // Outside discovery only the names a Refresh read count.
    let listed = covered(None);
    assert!(!changed(&listed, "src/b.h", Some(false)));
    assert!(!changed(&listed, "lib", Some(true)));
}

#[test]
fn a_directory_above_covered_names_made_or_removed_means_watching_afresh() {
    let mut watch = covered(None);
    watch.names.insert(b"src/a.c".to_vec());
    watch.directories.insert(b"src".to_vec());
    let root = watch.rule.root.clone();
    assert!(watch.replaced(&root.join("src"), Kind::Removed));
    assert!(watch.replaced(&root.join("src"), Kind::Added));
    assert!(!watch.replaced(&root.join("src"), Kind::Changed));
    assert!(!watch.replaced(&root.join("src/a.c"), Kind::Removed));
    assert!(!watch.replaced(&root.join("lib"), Kind::Added));
}

#[cfg(target_os = "linux")]
#[test]
fn a_change_made_while_a_refresh_reads_is_reported_after_it() {
    let work = tempfile::tempdir().expect("directory");
    let root = work.path().to_path_buf();
    std::fs::write(root.join("a.c"), "int Alpha;\n").expect("source");
    let selection = Selection {
        root: root.clone(),
        sources: Sources::Files(vec![b"a.c".to_vec()]),
        include_dirs: Vec::new(),
        environment_dirs: Vec::new(),
        system_includes: false,
        system_dir: b"/usr/include".to_vec(),
    };
    let stat = FileStat {
        size: 0,
        modified: 0,
        changed: 0,
        identity: Vec::new(),
    };
    let files = [StoredFile {
        path: b"a.c".to_vec(),
        unit: 1,
        stat,
        hash: [0; 32],
        verified: 0,
    }];
    let mut watch = Watch::new(&root.join("i.db"));
    assert!(watch.changed());
    watch.starting(&selection, &files, None);
    // After the Refresh read the file, before it finished.
    std::fs::write(root.join("a.c"), "int Beta;\n").expect("edit");
    watch.finished(1, &files, &[], &[]);
    assert!(watch.changed());
    watch.starting(&selection, &files, None);
    watch.finished(1, &files, &[], &[]);
    assert!(!watch.changed());
    // Only another revision read than the one the Refresh left means another process.
    let read = |number| RevisionId {
        identity: b"index".to_vec(),
        number,
    };
    watch.read(Some(&read(1)));
    assert!(!watch.changed());
    watch.read(Some(&read(2)));
    assert!(watch.changed());
}

/// What a watch is told about an indexed file: only its path matters to it.
fn stored(path: &str) -> StoredFile {
    StoredFile {
        path: path.as_bytes().to_vec(),
        unit: 1,
        stat: FileStat {
            size: 0,
            modified: 0,
            changed: 0,
            identity: Vec::new(),
        },
        hash: [0; 32],
        verified: 0,
    }
}

/// A Refresh of `root` that read `files` and found `missing` absent, as `watch` sees it.
fn refreshed(watch: &mut Watch, root: &Path, files: &[&str], missing: &[&str]) {
    let selection = Selection {
        root: root.to_path_buf(),
        sources: Sources::Files(Vec::new()),
        include_dirs: Vec::new(),
        environment_dirs: Vec::new(),
        system_includes: false,
        system_dir: b"/usr/include".to_vec(),
    };
    let files: Vec<StoredFile> = files.iter().map(|path| stored(path)).collect();
    let missing: Vec<Vec<u8>> = missing
        .iter()
        .map(|name| name.as_bytes().to_vec())
        .collect();
    watch.starting(&selection, &files, None);
    watch.finished(1, &files, &[], &missing);
}

/// A watch on `root` after two such Refreshes, so that it covered everything they met
/// before the second one read it.
fn covering(root: &Path, files: &[&str], missing: &[&str]) -> Watch {
    let mut watch = Watch::new(&root.join("i.db"));
    refreshed(&mut watch, root, files, missing);
    refreshed(&mut watch, root, files, missing);
    watch
}

/// Makes `outside` and the `directories` below it ones no watch reaches, as on Windows
/// any directory outside the root already is. On Linux they can then be searched and
/// written but not listed. `false` where permissions do not apply to this user.
fn unwatchable(outside: &Path, directories: &[&str]) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let unlisted = std::fs::Permissions::from_mode(0o311);
        std::fs::set_permissions(outside, unlisted.clone()).unwrap();
        for directory in directories {
            std::fs::set_permissions(outside.join(directory), unlisted.clone()).unwrap();
        }
        std::fs::read_dir(outside).is_err()
    }
    #[cfg(not(unix))]
    {
        let _ = (outside, directories);
        true
    }
}

/// Lets `outside` and the directories in it be listed again, so they can be removed.
fn listable(outside: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let open = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(outside, open.clone()).unwrap();
        for entry in std::fs::read_dir(outside).unwrap().flatten() {
            if entry.path().is_dir() {
                std::fs::set_permissions(entry.path(), open.clone()).unwrap();
            }
        }
    }
    let _ = outside;
}

#[test]
fn headers_made_removed_renamed_or_edited_where_no_watch_reaches_are_noticed() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let headers = work.path().join("headers");
    std::fs::create_dir(&root).unwrap();
    let cases = [
        "made", "removed", "within", "into", "saved", "edited", "below", "quiet",
    ];
    for case in cases {
        std::fs::create_dir_all(headers.join(case)).unwrap();
    }
    std::fs::create_dir(work.path().join("elsewhere")).unwrap();
    for (path, text) in [
        ("headers/removed/gone.h", "int Gone;\n"),
        ("headers/within/draft.h", "int Draft;\n"),
        ("elsewhere/late.h", "int Moved;\n"),
        ("headers/saved/saved.h", "int Old;\n"),
        ("headers/edited/edited.h", "int Old;\n"),
        ("headers/quiet/here.h", "int Here;\n"),
    ] {
        std::fs::write(work.path().join(path), text).unwrap();
    }
    if !unwatchable(&headers, &cases) {
        eprintln!("skipped: permissions do not apply to this user");
        return;
    }
    // Seen more than two seconds after they were last changed, the directories and
    // headers are checked by what they were then.
    std::thread::sleep(std::time::Duration::from_millis(2100));
    // Each change is made in a directory of its own, which a watch of its own covers.
    // Enough names are looked for in vain there that the directory is looked at by its
    // times.
    let noticed = |found: &[&str], missing: &[&str], change: &dyn Fn()| {
        let directory = missing.iter().chain(found).next().unwrap();
        let directory = &directory[..directory.rfind('/').unwrap()];
        let mut missing: Vec<String> = missing.iter().map(|name| name.to_string()).collect();
        missing.extend((0..MISSING_FOR_TIMES).map(|at| format!("{directory}/absent{at}.h")));
        let missing: Vec<&str> = missing.iter().map(String::as_str).collect();
        let mut watch = covering(&root, found, &missing);
        assert!(!watch.changed(), "changed before {found:?} {missing:?}");
        change();
        watch.changed()
    };
    let made = noticed(&[], &["../headers/made/late.h"], &|| {
        std::fs::write(headers.join("made/late.h"), "int Late;\n").unwrap();
    });
    assert!(made, "made");
    let removed = noticed(&["../headers/removed/gone.h"], &[], &|| {
        std::fs::remove_file(headers.join("removed/gone.h")).unwrap();
    });
    assert!(removed, "removed");
    let within = noticed(&[], &["../headers/within/late.h"], &|| {
        let draft = headers.join("within/draft.h");
        std::fs::rename(draft, headers.join("within/late.h")).unwrap();
    });
    assert!(within, "renamed within its directory");
    let into = noticed(&[], &["../headers/into/late.h"], &|| {
        let late = work.path().join("elsewhere/late.h");
        std::fs::rename(late, headers.join("into/late.h")).unwrap();
    });
    assert!(into, "renamed into its directory");
    // An editor's save: the new text goes to another file renamed over the header.
    let saved = noticed(&["../headers/saved/saved.h"], &[], &|| {
        let new = headers.join("saved/saved.h.new");
        std::fs::write(&new, "int New;\n").unwrap();
        std::fs::rename(new, headers.join("saved/saved.h")).unwrap();
    });
    assert!(saved, "saved by rename");
    let edited = noticed(&["../headers/edited/edited.h"], &[], &|| {
        std::fs::write(headers.join("edited/edited.h"), "int Newer;\n").unwrap();
    });
    assert!(edited, "edited");
    let later = noticed(&[], &["../headers/later/late.h"], &|| {
        std::fs::create_dir(headers.join("later")).unwrap();
        std::fs::write(headers.join("later/late.h"), "int Late;\n").unwrap();
    });
    assert!(later, "directory made");
    let below = noticed(
        &[],
        &["../headers/below/a.h", "../headers/below/sys/a.h"],
        &|| {
            std::fs::create_dir(headers.join("below/sys")).unwrap();
            std::fs::write(headers.join("below/sys/a.h"), "int Below;\n").unwrap();
        },
    );
    assert!(below, "directory made below one that is there");
    let quiet = noticed(
        &["../headers/quiet/here.h"],
        &["../headers/quiet/none.h"],
        &|| {},
    );
    assert!(!quiet, "nothing changed");
    listable(&headers);
}

#[test]
fn directories_with_many_names_missing_are_looked_at_by_their_times() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let headers = work.path().join("headers");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir_all(headers.join("many")).unwrap();
    std::fs::create_dir_all(headers.join("few")).unwrap();
    std::fs::write(headers.join("many/here.h"), "int Here;\n").unwrap();
    if !unwatchable(&headers, &["many", "few"]) {
        eprintln!("skipped: permissions do not apply to this user");
        return;
    }
    let mut missing: Vec<String> = (0..=MISSING_FOR_TIMES)
        .map(|at| format!("../headers/many/a{at}.h"))
        .collect();
    missing.push("../headers/many/sys/e.h".into());
    missing.push("../headers/few/x.h".into());
    missing.push("../headers/few/y.h".into());
    let missing: Vec<&str> = missing.iter().map(String::as_str).collect();
    let mut watch = covering(&root, &["../headers/many/here.h"], &missing);
    let looks = |watch: &Watch| watch.covered.as_ref().unwrap().polling.looks();
    // `many` by its times and the one header found there. `many/sys` is missing, and
    // made it would move the times of `many`. The two names missing in `few` are looked
    // for themselves.
    assert_eq!(looks(&watch), 4);
    std::fs::write(headers.join("many/a0.h"), "int A;\n").unwrap();
    // The next Refresh finds the new header, which is looked at from then on.
    refreshed(&mut watch, &root, &["../headers/many/here.h"], &missing);
    assert_eq!(looks(&watch), 5);
    listable(&headers);
}

/// A directory the watch cannot look at itself, as under a Windows folder that denies
/// reading its attributes but lets the names in it be looked up, has its names looked at.
#[test]
fn a_directory_that_cannot_be_looked_at_has_its_names_looked_at() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    let headers = work.path().join("headers");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&headers).unwrap();
    let mut polling = Polling::default();
    for at in 0..=MISSING_FOR_TIMES {
        polling.add(
            &root,
            b"../headers",
            format!("../headers/h{at}.h").as_bytes(),
        );
    }
    assert_eq!(polling.directories[0].by, By::Times);
    for seen in [
        Seen::Failed(std::io::ErrorKind::PermissionDenied),
        Seen::Other,
    ] {
        polling.directories[0].itself.seen = seen;
        polling.directories[0].by = polling.by(&polling.directories[0]);
        assert_eq!(polling.directories[0].by, By::Names);
    }
    assert!(!polling.changed());
    std::fs::write(headers.join("h0.h"), "int H;\n").unwrap();
    assert!(polling.changed());
}

#[test]
fn directories_spelled_through_the_root_are_known_as_the_root() {
    let root = if cfg!(windows) {
        PathBuf::from(r"C:\work\project")
    } else {
        PathBuf::from("/work/project")
    };
    assert!(spells_root(&root, b""));
    assert!(spells_root(&root, b"../project"));
    assert!(spells_root(&root, b"../project/sub/.."));
    assert!(!spells_root(&root, b"../project/sub"));
    assert!(!spells_root(&root, b"../other"));
    assert!(!spells_root(&root, b".."));
    assert_eq!(spells_root(&root, b"../Project"), cfg!(windows));
}

/// A directory link to the root, as a junction, a substituted drive or a short name can
/// also be on Windows, leads to the root's times all the same.
#[test]
fn a_directory_that_leads_to_the_root_is_looked_at_by_its_names() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    std::fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&root, work.path().join("alias")).unwrap();
    #[cfg(windows)]
    {
        let linked = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", "alias", "project"])
            .current_dir(work.path())
            .output()
            .unwrap();
        assert!(linked.status.success(), "{linked:?}");
    }
    let mut polling = Polling::default();
    for at in 0..=MISSING_FOR_TIMES {
        polling.add(&root, b"../alias", format!("../alias/h{at}.h").as_bytes());
    }
    assert!(polling.directories[0].root);
    assert_eq!(polling.directories[0].by, By::Names);
    polling.add(&root, b"../project/..", b"../project/../h.h");
    assert!(!polling.directories[1].root);
    // Where the root's identity cannot be read, any directory might be the root.
    let mut unknown = Polling {
        root_identity: Some(None),
        ..Polling::default()
    };
    unknown.add(&root, b"..", b"../h.h");
    assert!(unknown.directories[0].root);
}

/// The root holds the marker file the Windows watch makes before each query: a name
/// spelled through it is looked at itself, and the root's times are not.
#[cfg(windows)]
#[test]
fn names_spelled_through_the_root_are_looked_at_themselves() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let missing: Vec<String> = (0..=MISSING_FOR_TIMES)
        .map(|at| format!("../project/late{at}.h"))
        .collect();
    let missing: Vec<&str> = missing.iter().map(String::as_str).collect();
    let mut watch = covering(&root, &[], &missing);
    for _ in 0..3 {
        assert!(!watch.changed());
        watch.tidy();
    }
    std::fs::write(root.join("late0.h"), "int Late;\n").unwrap();
    assert!(watch.changed());
}

#[test]
fn a_watch_that_cannot_work_says_why_once() {
    let mut watch = Watch::new(Path::new("i.db"));
    watch.blind = Some("no watch here".into());
    assert!(watch.changed());
    assert_eq!(
        watch.note().as_deref(),
        Some(
            "cannot watch the sources for changes (no watch here); each query updates the index first"
        )
    );
    assert_eq!(watch.note(), None);
    assert!(watch.changed());
}
