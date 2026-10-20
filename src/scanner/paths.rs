//! The reference canonicalises every include operand before storing it (reading-c.md §6),
//! so the displayed operand and the Include name use the canonical form
//! [24-include-operands].

/// The operand in canonical form. An operand without `/` is unchanged. Otherwise, read
/// as `/`-separated names:
/// - empty names go, so `//` becomes `/`;
/// - `.` goes, unless it is the last name;
/// - `..` cancels the name before it, unless that name is `..` too;
/// - a leading `/` stays, a trailing `/` goes, and a trailing `/.` goes unless it is the
///   whole path;
/// - an operand left empty becomes `.`.
pub(super) fn canonical(operand: &[u8]) -> Vec<u8> {
    if !operand.contains(&b'/') {
        return operand.to_vec();
    }
    let names: Vec<&[u8]> = operand
        .split(|byte| *byte == b'/')
        .filter(|name| !name.is_empty())
        .collect();
    let open_end = operand.last() != Some(&b'/');
    let mut kept: Vec<&[u8]> = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let last = open_end && index + 1 == names.len();
        match *name {
            b"." if !last => {}
            b".." if kept.last().is_some_and(|before| *before != b"..") => {
                kept.pop();
            }
            _ => kept.push(name),
        }
    }
    let mut path = Vec::with_capacity(operand.len());
    if operand[0] == b'/' {
        path.push(b'/');
    }
    path.extend(kept.join(&b'/'));
    if path.len() > 2 && path.ends_with(b"/.") {
        path.truncate(path.len() - 2);
    }
    if path.is_empty() {
        path.push(b'.');
    }
    path
}

#[cfg(test)]
mod tests {
    use super::canonical;

    #[test]
    fn operands_match_the_reference_probe() {
        for (operand, expected) in [
            ("./local.h", "local.h"),
            ("dir//double.h", "dir/double.h"),
            ("a/../b.h", "b.h"),
            ("sys//angled.h", "sys/angled.h"),
            ("../up.h", "../up.h"),
            ("x/.", "x"),
            ("a/b/../../c.h", "c.h"),
            ("//'SYS1.SAMPLIB(CSRSIC)'", "/'SYS1.SAMPLIB(CSRSIC)'"),
            ("././twice.h", "twice.h"),
            ("dir/", "dir"),
            ("./", "."),
            ("../../deep/./x.h", "../../deep/x.h"),
            ("a/./../b.h", "b.h"),
            ("plain.h", "plain.h"),
        ] {
            assert_eq!(
                canonical(operand.as_bytes()),
                expected.as_bytes(),
                "{operand}"
            );
        }
    }

    #[test]
    fn edge_operands_keep_their_verified_forms() {
        // Checked against the earlier pass-by-pass implementation on every operand of up
        // to nine bytes over `a`, `.` and `/`.
        for (operand, expected) in [
            ("/", "/"),
            ("/.", "/."),
            ("/..", "/.."),
            ("/../a", "/../a"),
            ("/a/..", "/"),
            ("a/..", "."),
            ("a/../", "."),
            ("../..", "../.."),
            ("../.", ".."),
            ("./.", "."),
            (".a/..", "."),
            (".../..", "."),
            ("a/./.", "a"),
            ("a//..//b", "b"),
            ("/a/../..", "/.."),
        ] {
            assert_eq!(
                canonical(operand.as_bytes()),
                expected.as_bytes(),
                "{operand}"
            );
        }
    }
}
