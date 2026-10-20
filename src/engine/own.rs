//! Ripscope's own query semantics for fields 0, 2, 3 and 9 (ADR 0010): names are read in
//! order as by cscope's sequential search, but each name's context is worked out within
//! its file, whether or not an earlier row on its line matched, and nothing carries from
//! one file to the next.

use super::{Context, Indexed, Matcher, Row, Rows, Stored, lines};
use crate::unit::{Mark, Scope};

/// The macro a name sits in, else its function.
fn context(scope: Scope<'_>) -> Option<&[u8]> {
    scope.macro_name.or(scope.function)
}

/// Reads the names of a file, or of a block of its lines, in order, from the scope it
/// starts in, giving each with the scope it sits in.
fn walk<'u>(indexed: &Indexed<'u>, mut visit: impl FnMut(&Stored<'u, '_>, usize, Scope<'u>)) {
    let mut scope = indexed.start;
    let mut lines = lines(indexed.unit);
    while let Some(stored) = lines.next() {
        for (index, name) in stored.names.iter().enumerate() {
            scope.read(name);
            visit(&stored, index, scope);
        }
        scope.leave(&stored.line);
    }
}

/// Fields 0 and 9: at most one row per line, for its first matching name (for field 9,
/// its first matching assigned name), with that name's context. A macro definition's own
/// row takes the function it sits in.
pub(super) fn symbols<'u>(
    matcher: &mut Matcher<'_>,
    files: &[Indexed<'u>],
    assignments: bool,
    rows: &mut Rows<'u>,
) {
    // Select once so the occurrence walk needs no runtime search-kind flag.
    if assignments {
        symbols_for::<true>(matcher, files, rows);
    } else {
        symbols_for::<false>(matcher, files, rows);
    }
}

fn symbols_for<'u, const ASSIGNMENTS: bool>(
    matcher: &mut Matcher<'_>,
    files: &[Indexed<'u>],
    rows: &mut Rows<'u>,
) {
    for indexed in files {
        let mut answered = false;
        walk(indexed, |stored, index, scope| {
            if index == 0 {
                answered = false;
            }
            let name = stored.names[index];
            if answered || (ASSIGNMENTS && !name.assigned) {
                return;
            }
            let matched = match name.mark {
                Mark::FunctionEnd | Mark::DefineEnd | Mark::Include => false,
                Mark::Plain => matcher.plain(name.text),
                _ => matcher.marked(name.text),
            };
            if !matched {
                return;
            }
            let context = if name.mark == Mark::Define {
                scope.function
            } else {
                context(scope)
            };
            rows.push(Row {
                path: indexed.path,
                context: context.map_or(Context::Global, Context::Name),
                line: stored.line.number,
                text: stored.text(index),
            });
            answered = true;
        });
    }
}

/// Field 3: each matching call, attributed to the macro it sits in, else to its function.
/// A call outside both gives no row.
pub(super) fn callers<'u>(matcher: &mut Matcher<'_>, files: &[Indexed<'u>], rows: &mut Rows<'u>) {
    for indexed in files {
        walk(indexed, |stored, index, scope| {
            let name = stored.names[index];
            if name.mark != Mark::Call || !matcher.marked(name.text) {
                return;
            }
            if let Some(caller) = context(scope) {
                rows.push(Row {
                    path: indexed.path,
                    context: Context::Name(caller),
                    line: stored.line.number,
                    text: stored.text(index),
                });
            }
        });
    }
}

/// Field 2: each call whose macro, else function, matches, with the called name as its
/// context.
pub(super) fn callees<'u>(matcher: &mut Matcher<'_>, files: &[Indexed<'u>], rows: &mut Rows<'u>) {
    for indexed in files {
        // The last context asked about and its answer: a body's calls share one, which
        // `ptr::eq` recognises by its address and length.
        let mut asked: Option<(&'u [u8], bool)> = None;
        walk(indexed, |stored, index, scope| {
            let name = stored.names[index];
            if name.mark != Mark::Call {
                return;
            }
            let Some(caller) = context(scope) else {
                return;
            };
            let matched = match asked {
                Some((last, matched)) if std::ptr::eq(last, caller) => matched,
                _ => {
                    let matched = matcher.marked(caller);
                    asked = Some((caller, matched));
                    matched
                }
            };
            if matched {
                rows.push(Row {
                    path: indexed.path,
                    context: Context::Name(name.text),
                    line: stored.line.number,
                    text: stored.text(index),
                });
            }
        });
    }
}

#[cfg(test)]
mod tests;
