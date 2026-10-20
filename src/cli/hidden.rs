//! What the hidden developer commands share.

use crate::analyzer::Analyzer;
use crate::scanner::Corrections;
use std::ffi::OsString;

/// The Analyzer `--corrections NAMES` asks for (comma-separated Correction names, `all`
/// or `enabled`), wherever it is given (the last one counts), and the other arguments.
/// Without it the commands read as the reference does, which the compass compares with
/// cscope.
pub(super) fn analyzer(args: &[OsString]) -> Result<(Analyzer, Vec<OsString>), String> {
    let mut corrections = Corrections::default();
    let mut rest = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--corrections" {
            let names = args.next().ok_or("--corrections needs names")?;
            corrections = Corrections::parse(&names.to_string_lossy())?;
        } else {
            rest.push(arg.clone());
        }
    }
    Ok((Analyzer::with(corrections), rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<(Analyzer, Vec<OsString>), String> {
        analyzer(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn corrections_are_taken_wherever_they_are_given() {
        let (reference, rest) = parse(&["-i", "list"]).unwrap();
        assert_eq!(reference, Analyzer::with(Corrections::default()));
        assert_eq!(rest, ["-i", "list"]);
        for args in [
            &["--corrections", "enabled", "-i", "list"][..],
            &["-i", "list", "--corrections", "enabled"],
        ] {
            let (analyzer, rest) = parse(args).unwrap();
            assert_eq!(analyzer, Analyzer::PRODUCT, "{args:?}");
            assert_eq!(rest, ["-i", "list"], "{args:?}");
        }
        assert_eq!(
            parse(&["-i", "list", "--corrections"]).unwrap_err(),
            "--corrections needs names"
        );
        assert!(parse(&["--corrections", "no-such-correction"]).is_err());
        let (last, _) = parse(&["--corrections", "enabled", "--corrections", ""]).unwrap();
        assert_eq!(last, Analyzer::with(Corrections::default()));
    }
}
