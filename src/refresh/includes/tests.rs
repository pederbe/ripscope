use super::*;
use crate::refresh::Sources;
use std::path::Path;

fn write(root: &Path, name: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"#define API(type) type\n").unwrap();
}

fn selection(root: &Path, directories: &[&str]) -> Selection {
    Selection {
        root: root.to_owned(),
        sources: Sources::Files(vec![b"src/root.c".to_vec()]),
        include_dirs: directories.iter().map(|d| d.as_bytes().to_vec()).collect(),
        environment_dirs: vec![b"environment".to_vec()],
        system_includes: true,
        system_dir: b"system".to_vec(),
    }
}

#[test]
fn discovery_keeps_directory_precedence_and_quoted_build_root_lookup() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    for name in [
        "api.h",
        "src/api.h",
        "command/api.h",
        "listed/api.h",
        "environment/api.h",
        "system/api.h",
        "listed/list.h",
        "environment/list.h",
        "system/list.h",
        "environment/env.h",
        "system/env.h",
        "system/sys.h",
    ] {
        write(root, name);
    }
    let rule = selection(root, &["command"]);
    let mut resolver = IncludeResolver::new(&rule, &[b"listed".to_vec()], true);
    let listed = HashSet::default();
    assert_eq!(
        resolver.discover(b"./api.h", true, &listed),
        Some(b"api.h".to_vec())
    );
    for (operand, expected) in [
        (b"api.h".as_slice(), b"command/api.h".as_slice()),
        (b"list.h", b"listed/list.h"),
        (b"env.h", b"environment/env.h"),
        (b"sys.h", b"system/sys.h"),
    ] {
        assert_eq!(
            resolver.discover(operand, false, &listed),
            Some(expected.to_vec())
        );
    }
    let mut kernel = IncludeResolver::new(&rule, &[], false);
    assert_eq!(kernel.discover(b"sys.h", false, &listed), None);
}

#[test]
fn an_already_listed_candidate_stops_discovery() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "first/api.h");
    write(root, "last/api.h");
    let rule = selection(root, &["first", "last"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    for name in [b"api.h".as_slice(), b"first/api.h"] {
        let listed = [name.to_vec()].into_iter().collect::<HashSet<_>>();
        for quoted in [true, false] {
            assert_eq!(resolver.discover(b"api.h", quoted, &listed), None);
        }
    }
    assert_eq!(
        resolver.discover(b"api.h", true, &HashSet::default()),
        Some(b"first/api.h".to_vec())
    );
}

#[test]
fn a_quoted_operand_is_found_beside_the_index_before_the_directories() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "api.h");
    write(root, "src/api.h");
    write(root, "include/api.h");
    let rule = selection(root, &["include"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    let listed = HashSet::default();
    assert_eq!(
        resolver.discover(b"api.h", true, &listed),
        Some(b"api.h".to_vec())
    );
    assert_eq!(
        resolver.discover(b"api.h", false, &listed),
        Some(b"include/api.h".to_vec())
    );
}

#[test]
fn discovery_passes_candidates_that_are_not_regular_files() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    std::fs::create_dir_all(root.join("first/api.h")).unwrap();
    write(root, "blocked");
    write(root, "last/api.h");
    let rule = selection(root, &["first", "blocked", "last"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    assert_eq!(
        resolver.discover(b"api.h", true, &HashSet::default()),
        Some(b"last/api.h".to_vec())
    );
}

#[test]
fn each_refresh_reconsiders_absent_candidates_and_directory_listings() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    let rule = selection(root, &["include"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    assert_eq!(resolver.discover(b"new.h", true, &HashSet::default()), None);
    write(root, "include/new.h");
    let mut next = IncludeResolver::new(&rule, &[], false);
    assert_eq!(
        next.discover(b"new.h", true, &HashSet::default()),
        Some(b"include/new.h".to_vec())
    );
}

/// An include directory is spelled lexically, so `..` after a link climbs from the
/// written name, as cscope's paths do.
#[test]
fn directories_through_links_are_spelled_lexically() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "include/api.h");
    write(root, "other/api.h");
    std::fs::create_dir_all(root.join("other/child")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("other/child"), root.join("include/link")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(root.join("other/child"), root.join("include/link")).unwrap();
    let rule = selection(root, &["include/link/.."]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    assert_eq!(
        resolver.discover(b"api.h", true, &HashSet::default()),
        Some(b"include/api.h".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn inaccessible_candidates_are_passed_over() {
    use std::os::unix::fs::PermissionsExt;
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "first/api.h");
    write(root, "last/api.h");
    let first = root.join("first");
    std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o000)).unwrap();
    let denied = std::fs::metadata(first.join("api.h"));
    let rule = selection(root, &["first", "last"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    let discovery = resolver.discover(b"api.h", true, &HashSet::default());
    std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        denied.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied,
        "run as an ordinary user"
    );
    assert_eq!(discovery, Some(b"last/api.h".to_vec()));
}

#[cfg(windows)]
#[test]
fn discovery_takes_only_exact_spellings() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "src/API.h");
    write(root, "include/api.h");
    let rule = selection(root, &["src", "include"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    assert_eq!(
        resolver.discover(b"api.h", true, &HashSet::default()),
        Some(b"include/api.h".to_vec())
    );
    assert_eq!(
        resolver.discover(b"API.h", true, &HashSet::default()),
        Some(b"src/API.h".to_vec())
    );
}

/// The questions a resolver records read back as it asked them, and a record an earlier
/// version made with other kinds of question is not read.
#[test]
fn recorded_questions_read_back_and_others_are_refused() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "include/api.h");
    let rule = selection(root, &["include"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    assert_eq!(
        resolver.discover(b"api.h", true, &HashSet::default()),
        Some(b"include/api.h".to_vec())
    );
    let observed = resolver.observations();
    assert_eq!(
        questions(&observed),
        Some(vec![(b"api.h".as_slice(), false), (b"include/api.h", true)])
    );
    assert!(IncludeResolver::new(&rule, &[], false).observes(&observed));
    std::fs::remove_file(root.join("include/api.h")).unwrap();
    assert!(!IncludeResolver::new(&rule, &[], false).observes(&observed));
    let mut earlier = observed.clone();
    earlier.extend_from_slice(b"c");
    earlier.extend_from_slice(&5u64.to_le_bytes());
    earlier.extend_from_slice(b"api.h\0");
    assert_eq!(questions(&earlier), None);
    assert!(!IncludeResolver::new(&rule, &[], false).observes(&earlier));
}

/// A large project's questions, asked on several threads, give the answers they gave
/// when recorded, and one changed answer anywhere among them is noticed.
#[test]
fn many_questions_are_answered_as_recorded() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    let names: Vec<_> = (0..700)
        .map(|at| format!("dir{}/api{at}.h", at % 7))
        .collect();
    for name in &names {
        write(root, name);
    }
    let rule = selection(root, &["include"]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    for name in &names {
        assert!(
            resolver
                .discover(name.as_bytes(), true, &HashSet::default())
                .is_some()
        );
    }
    assert!(
        resolver
            .discover(b"absent.h", true, &HashSet::default())
            .is_none()
    );
    let observed = resolver.observations();
    assert!(questions(&observed).unwrap().len() > 512);
    assert!(IncludeResolver::new(&rule, &[], false).observes(&observed));
    std::fs::remove_file(root.join(&names[350])).unwrap();
    assert!(!IncludeResolver::new(&rule, &[], false).observes(&observed));
    write(root, &names[350]);
    assert!(IncludeResolver::new(&rule, &[], false).observes(&observed));
    write(root, "include/absent.h");
    assert!(!IncludeResolver::new(&rule, &[], false).observes(&observed));
}

/// A Refresh that goes on after its early stop records only what its own discovery asks,
/// not the questions the early stop asked.
#[test]
fn a_failed_early_stop_leaves_no_questions_behind() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "include/old.h");
    write(root, "include/new.h");
    let rule = selection(root, &["include"]);
    let mut earlier = IncludeResolver::new(&rule, &[], false);
    assert!(
        earlier
            .discover(b"old.h", true, &HashSet::default())
            .is_some()
    );
    let observed = earlier.observations();
    std::fs::remove_file(root.join("include/old.h")).unwrap();
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    assert!(!resolver.observes(&observed));
    assert!(
        resolver
            .discover(b"new.h", true, &HashSet::default())
            .is_some()
    );
    let mut fresh = IncludeResolver::new(&rule, &[], false);
    assert!(
        fresh
            .discover(b"new.h", true, &HashSet::default())
            .is_some()
    );
    assert_eq!(resolver.observations(), fresh.observations());
}

/// On several threads a Windows question still asks for the exact spelling: a file whose
/// name now differs only in case no longer answers it.
#[cfg(windows)]
#[test]
fn many_questions_keep_exact_spellings() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    let names: Vec<_> = (0..600)
        .map(|at| format!("dir{}/api{at}.h", at % 5))
        .collect();
    for name in &names {
        write(root, name);
    }
    let rule = selection(root, &[]);
    let mut resolver = IncludeResolver::new(&rule, &[], false);
    for name in &names {
        assert!(
            resolver
                .discover(name.as_bytes(), true, &HashSet::default())
                .is_some()
        );
    }
    let observed = resolver.observations();
    assert!(IncludeResolver::new(&rule, &[], false).observes(&observed));
    std::fs::rename(root.join(&names[400]), root.join(names[400].to_uppercase())).unwrap();
    assert!(!IncludeResolver::new(&rule, &[], false).observes(&observed));
}
