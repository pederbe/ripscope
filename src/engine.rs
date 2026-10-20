//! The query engine: the stored query fields, answered over Analysis
//! units (docs/architecture/answering-queries.md) with one of two query semantics.
//!
//! A query says what it reads ([`Reads`]), and a [`Source`], such as an index revision,
//! gives it those files in database order. It reads each file's stored lines in order and
//! each line's names in order. Rows come out in cscope's order: file-scope rows first,
//! then the rest.

use crate::query::pattern::CompiledPattern;
use crate::unit::{AnalysisUnit, Line, Mark, Name, Scope, display_cut, resume_after, shown};

mod own;
#[cfg(test)]
mod tests;

/// How fields 0, 2, 3 and 9 work out a name's context (ADR 0010).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Semantics {
    /// Ripscope's own: each name's context is worked out within its file, and nothing
    /// carries from file to file.
    Own,
    /// The reference cscope's sequential search, kept exactly as the oracle: a row
    /// consumes the rest of its line, context changes there included, and some context
    /// carries from file to file.
    Sequential,
}

/// A stored query field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    /// 0: every occurrence of a name.
    Symbols,
    /// 1: global definitions.
    Definitions,
    /// 2: functions called by a function.
    Callees,
    /// 3: functions calling a function.
    Callers,
    /// 7: files whose path matches.
    Files,
    /// 8: files including a file.
    Includers,
    /// 9: assignments to a name.
    Assignments,
}

impl Field {
    pub(crate) fn from_digit(digit: u8) -> Option<Self> {
        Some(match digit {
            b'0' => Self::Symbols,
            b'1' => Self::Definitions,
            b'2' => Self::Callees,
            b'3' => Self::Callers,
            b'7' => Self::Files,
            b'8' => Self::Includers,
            b'9' => Self::Assignments,
            _ => return None,
        })
    }
}

/// An indexed file and its unit, or a block of its unit's lines (ADR 0011). A query reads
/// them in the order given, which is the database order, and a file's blocks in order.
#[derive(Clone, Copy)]
pub(crate) struct Indexed<'u> {
    pub path: &'u [u8],
    /// Its lines: the whole unit, or a block of it.
    pub unit: &'u AnalysisUnit,
    /// The own semantics' scope at its first line: the file scope for a whole unit.
    pub start: Scope<'u>,
    /// Whether it starts its file. The sequential semantics read files whole, and walk on
    /// across a file's blocks.
    pub first: bool,
}

/// Where a stored query's files come from: an index revision, or in the tests a source
/// that gives only the files a query needs.
pub(crate) trait Source {
    /// The paths it gives, in database order: every indexed path when it was read for
    /// field 7 ([`Reads::Paths`]).
    fn paths(&self) -> impl Iterator<Item = &[u8]>;

    /// The files `reads` asks for, in database order, each as its unit or as blocks of it,
    /// in order: at least every file that holds a name it asks for, and every file for
    /// [`Reads::Every`]. A file whose unit cannot be read is left out.
    fn read(&self, reads: &Reads<'_>) -> Vec<Indexed<'_>>;

    /// For an exact name, each file it read, in database order, with the blocks of its
    /// unit read and the unit's answer rows for the name; none when it has no rows.
    fn occurrences(&self) -> Option<Vec<Occurring<'_>>> {
        None
    }

    /// For field 8, each file whose unit holds a matching include, in database order, with
    /// its matching includes as the index recorded them; none when it has not recorded them.
    fn included(&self) -> Option<Vec<Including<'_>>> {
        None
    }
}

/// A file and its unit's matching includes, in order, each with its row.
pub(crate) type Including<'u> = (&'u [u8], &'u [crate::store::Included]);

/// A file, the blocks of its unit read, and the unit's answer rows for an exact name
/// ([`crate::unit::Occurrences`]).
pub(crate) type Occurring<'u> = (
    &'u [u8],
    &'u [std::sync::Arc<crate::store::Block>],
    &'u [u8],
);

/// What a stored query reads (docs/architecture/answering-queries.md, section 1). Under
/// the own semantics a file gives rows only through the names it holds, so a query that
/// reads names needs only the files that hold one it asks for; the sequential semantics
/// carry context from file to file, so their fields 0, 2, 3 and 9 read every file.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reads<'q> {
    /// Field 7: the indexed paths, and no unit.
    Paths,
    /// Every file with its unit.
    Every,
    /// The files that hold a name in one of `roles` that `pattern` matches.
    Names {
        roles: Roles,
        pattern: &'q CompiledPattern<'q>,
    },
    /// Field 8: the files that hold an include whose operand, without its opening
    /// delimiter, `pattern` matches.
    Includes { pattern: &'q CompiledPattern<'q> },
}

/// The names a field reads: those with one of its marks and, for field 9, only those
/// marked assigned. A plain name is tried against a regular expression only when it
/// starts with a letter or underscore, as the field reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Roles {
    marks: &'static [Mark],
    assigned: bool,
}

impl Roles {
    /// The roles of the names it reads, as the index's name postings keep them
    /// ([`Mark::role`]).
    pub(crate) fn mask(&self) -> u32 {
        self.marks.iter().fold(0, |mask, mark| {
            let unassigned = if self.assigned { 0 } else { mark.role(false) };
            mask | mark.role(true) | unassigned
        })
    }

    /// Which roles of each name the query reads, for a pattern matched against every
    /// name the index holds.
    pub(crate) fn wanted<'p>(self, pattern: &'p CompiledPattern<'p>) -> Wanted<'p> {
        Wanted {
            matcher: Matcher::new(pattern),
            mask: self.mask(),
        }
    }

    /// Field 0: every name but include operands and function and macro ends.
    const SYMBOLS: Self = Self {
        marks: &[
            Mark::Plain,
            Mark::Define,
            Mark::Function,
            Mark::Call,
            Mark::Global,
            Mark::Member,
            Mark::Struct,
            Mark::Union,
            Mark::Enum,
            Mark::Typedef,
            Mark::Class,
        ],
        assigned: false,
    };
    /// Field 9: field 0's names marked assigned.
    const ASSIGNMENTS: Self = Self {
        assigned: true,
        ..Self::SYMBOLS
    };
    /// Field 1: definitions.
    const DEFINITIONS: Self = Self {
        marks: &[
            Mark::Define,
            Mark::Function,
            Mark::Class,
            Mark::Enum,
            Mark::Member,
            Mark::Struct,
            Mark::Typedef,
            Mark::Union,
            Mark::Global,
        ],
        assigned: false,
    };
    /// Field 2: the function and macro heads a call can sit in, not the calls.
    const HEADS: Self = Self {
        marks: &[Mark::Define, Mark::Function],
        assigned: false,
    };
    /// Field 3: calls.
    const CALLS: Self = Self {
        marks: &[Mark::Call],
        assigned: false,
    };
}

/// Which roles of a name a query reads, as the index's name postings keep them
/// ([`Mark::role`]): those of its field, for a name its pattern matches. A regular
/// expression tries a plain name only when it starts with a letter or underscore.
pub(crate) struct Wanted<'p> {
    matcher: Matcher<'p>,
    mask: u32,
}

impl Wanted<'_> {
    /// The roles of `name` the query reads: none when its pattern does not match it.
    pub(crate) fn roles(&mut self, name: &[u8]) -> u32 {
        if !self.matcher.marked(name) {
            0
        } else if self.matcher.tries_plain(name) {
            self.mask
        } else {
            self.mask & !(Mark::Plain.role(false) | Mark::Plain.role(true))
        }
    }
}

/// What field `field` reads with `semantics` for `pattern`.
pub(crate) fn reads<'q>(
    semantics: Semantics,
    field: Field,
    pattern: &'q CompiledPattern<'q>,
) -> Reads<'q> {
    let names = |roles| Reads::Names { roles, pattern };
    match (semantics, field) {
        (_, Field::Files) => Reads::Paths,
        (_, Field::Includers) => Reads::Includes { pattern },
        (_, Field::Definitions) => names(Roles::DEFINITIONS),
        (Semantics::Sequential, _) => Reads::Every,
        (Semantics::Own, Field::Symbols) => names(Roles::SYMBOLS),
        (Semantics::Own, Field::Assignments) => names(Roles::ASSIGNMENTS),
        (Semantics::Own, Field::Callees) => names(Roles::HEADS),
        (Semantics::Own, Field::Callers) => names(Roles::CALLS),
    }
}

#[cfg(test)]
impl Reads<'_> {
    /// Whether `unit` holds what the query reads. A file whose unit does not gives no row,
    /// which answering over only the files that hold it relies on; the tests check that
    /// against answers over every file.
    pub(crate) fn holds(&self, unit: &AnalysisUnit) -> bool {
        let names = |wanted: &mut dyn FnMut(Name<'_>) -> bool| {
            unit.lines().any(|line| line.names().any(&mut *wanted))
        };
        match *self {
            Self::Paths => false,
            Self::Every => true,
            Self::Names { roles, pattern } => {
                let mut matcher = Matcher::new(pattern);
                names(&mut |name| {
                    roles.marks.contains(&name.mark)
                        && (name.assigned || !roles.assigned)
                        && if name.mark == Mark::Plain {
                            matcher.plain(name.text)
                        } else {
                            matcher.marked(name.text)
                        }
                })
            }
            Self::Includes { pattern } => {
                let mut matcher = Matcher::new(pattern);
                names(&mut |name| {
                    name.mark == Mark::Include
                        && matcher.marked(name.text.get(1..).unwrap_or_default())
                })
            }
        }
    }
}

/// What a row's context column shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Context<'u> {
    /// `<global>`: the file scope.
    Global,
    /// The enclosing function or macro; for field 2, the function called.
    Name(&'u [u8]),
    /// Field 1 shows the pattern itself.
    Pattern,
    /// Field 7 shows `<unknown>`.
    Unknown,
}

/// One result row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row<'u> {
    /// The indexed path.
    pub path: &'u [u8],
    pub context: Context<'u>,
    pub line: u32,
    /// The displayed text, before any transport escaping.
    pub text: &'u [u8],
}

impl Row<'_> {
    /// Whether the row is at the file scope (`<global>`): an answer gives those rows
    /// first.
    pub(crate) fn at_file_scope(&self) -> bool {
        self.context == Context::Global
    }
}

/// Field 7's placeholder text.
pub(crate) const UNKNOWN: &[u8] = b"<unknown>";

/// Answers one stored query from `source` with `semantics`, returning its rows in cscope's
/// order: field 7 from the indexed paths, the others from the files the query reads
/// ([`reads`]). Fields 1, 7 and 8 are the same under both.
pub(crate) fn answer<'u>(
    semantics: Semantics,
    field: Field,
    pattern: &CompiledPattern<'_>,
    source: &'u impl Source,
) -> Vec<Row<'u>> {
    let mut matcher = Matcher::new(pattern);
    let mut rows = Rows::default();
    if let Some(answered) = {
        profile_span!("recorded");
        recorded(semantics, field, pattern, source)
    } {
        profile_count!("recorded_rows", answered.len());
        return answered;
    }
    let read = || source.read(&reads(semantics, field, pattern));
    match (semantics, field) {
        (Semantics::Own, Field::Symbols) => own::symbols(&mut matcher, &read(), false, &mut rows),
        (Semantics::Own, Field::Assignments) => {
            own::symbols(&mut matcher, &read(), true, &mut rows);
        }
        (Semantics::Own, Field::Callees) => own::callees(&mut matcher, &read(), &mut rows),
        (Semantics::Own, Field::Callers) => own::callers(&mut matcher, &read(), &mut rows),
        (Semantics::Sequential, Field::Symbols) => {
            symbols(&mut matcher, &read(), false, &mut rows);
        }
        (Semantics::Sequential, Field::Assignments) => {
            symbols(&mut matcher, &read(), true, &mut rows);
        }
        (Semantics::Sequential, Field::Callees) => callees(&mut matcher, &read(), &mut rows),
        (Semantics::Sequential, Field::Callers) => callers(&mut matcher, &read(), &mut rows),
        (_, Field::Definitions) => definitions(&mut matcher, &read(), &mut rows),
        (_, Field::Files) => {
            for path in source.paths() {
                if matcher.request.matches_name(path, &mut matcher.folded) {
                    rows.push(Row {
                        path,
                        context: Context::Unknown,
                        line: 1,
                        text: UNKNOWN,
                    });
                }
            }
        }
        (_, Field::Includers) => match source.included() {
            Some(files) => return placed(&files),
            None => includers(&mut matcher, &read(), &mut rows),
        },
    }
    rows.finish()
}

/// The answer of an exact name's query in fields 0, 1, 2, 3 and 9 from the rows the
/// index recorded for it, which give each row's line, name and context, so that only those
/// lines are read; none where there are no rows for it or they are damaged (the blocks
/// are walked instead).
fn recorded<'u>(
    semantics: Semantics,
    field: Field,
    pattern: &CompiledPattern<'_>,
    source: &'u impl Source,
) -> Option<Vec<Row<'u>>> {
    let digit = match (semantics, field) {
        (_, Field::Definitions) => 1,
        (Semantics::Own, Field::Symbols) => 0,
        (Semantics::Own, Field::Callees) => 2,
        (Semantics::Own, Field::Callers) => 3,
        (Semantics::Own, Field::Assignments) => 9,
        _ => return None,
    };
    pattern.exact_name()?;
    let files = source.occurrences()?;
    let mut rows = Rows::default();
    let mut names = Vec::new();
    for (path, blocks, recorded) in files {
        for occurrence in crate::unit::Occurrences(recorded).iter() {
            let occurrence = occurrence?;
            if occurrence.field != digit {
                continue;
            }
            let block = blocks
                .iter()
                .find(|block| block.number == occurrence.block)?;
            let line = block.lines.line_at(occurrence.offset, occurrence.line)?;
            names.clear();
            names.extend(line.names());
            let index = usize::try_from(occurrence.index).ok()?;
            let name = *names.get(index)?;
            let cut = display_cut(&names);
            let stored = Stored {
                line,
                names: &names,
                cut,
            };
            let context = match digit {
                1 => Context::Pattern,
                2 => Context::Name(name.text),
                _ => match occurrence.context {
                    Some(within) => Context::Name(within.name(&block.lines, block.start())?),
                    None => Context::Global,
                },
            };
            rows.push(Row {
                path,
                context,
                line: line.number,
                text: stored.text(index),
            });
        }
    }
    Some(rows.finish())
}

/// Field 8 from the rows the index recorded for the matching includes, which reads no
/// lines: as [`includers`] answers, each row consuming the rest of its line, so that a
/// later match on the same line before where reading resumes gives no row.
fn placed<'u>(files: &[Including<'u>]) -> Vec<Row<'u>> {
    let mut rows = Rows::default();
    for &(path, included) in files {
        // The line of the last row, and the name reading resumes at after it.
        let mut resume = None;
        for included in included {
            let (line, index) = (included.at.line, included.at.name);
            if resume.is_some_and(|(number, after)| number == line && index < after) {
                continue;
            }
            resume = Some((line, included.after));
            rows.push(Row {
                path,
                context: Context::Global,
                line,
                text: &included.text,
            });
        }
    }
    rows.finish()
}

/// cscope collects rows at the file scope first and the rest after them, each group in
/// the order found.
#[derive(Default)]
struct Rows<'u> {
    global: Vec<Row<'u>>,
    other: Vec<Row<'u>>,
}

impl<'u> Rows<'u> {
    fn push(&mut self, row: Row<'u>) {
        if row.at_file_scope() {
            self.global.push(row);
        } else {
            self.other.push(row);
        }
    }

    fn finish(mut self) -> Vec<Row<'u>> {
        self.global.append(&mut self.other);
        self.global
    }
}

/// The compiled pattern, as the engine asks it about names.
struct Matcher<'p> {
    request: &'p CompiledPattern<'p>,
    /// Matched as a regular expression (a regex pattern, or any pattern with `-C`).
    regex: bool,
    caseless: bool,
    folded: Vec<u8>,
}

impl<'p> Matcher<'p> {
    fn new(request: &'p CompiledPattern<'p>) -> Self {
        Self {
            request,
            regex: request.exact_name().is_none(),
            caseless: request.is_case_insensitive(),
            folded: Vec::new(),
        }
    }

    /// A name with a mark: exact names compare whole, and regular expressions never
    /// match an empty name.
    fn marked(&mut self, name: &[u8]) -> bool {
        !name.is_empty() && self.request.matches_name(name, &mut self.folded)
    }

    /// A plain name, which cscope's search reads as an unmarked line: a regular
    /// expression is tried only when the line starts with a letter or underscore.
    fn plain(&mut self, name: &[u8]) -> bool {
        self.tries_plain(name) && self.marked(name)
    }

    /// Whether a plain name is tried at all.
    fn tries_plain(&self, name: &[u8]) -> bool {
        !self.regex
            || name
                .first()
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
    }
}

/// A stored line as a query reads it, its names held by the [`Lines`] it comes from.
struct Stored<'u, 'n> {
    line: Line<'u>,
    names: &'n [Name<'u>],
    /// The name after which cscope's display of the line stops: one followed directly by
    /// another name, which its database separates by an empty fragment.
    cut: Option<usize>,
}

impl<'u> Stored<'u, '_> {
    /// The text a row for the name at `index` shows. cscope shows the line up to the
    /// cut; a match after the cut aborts its query, and ripscope shows the whole line.
    fn text(&self, index: usize) -> &'u [u8] {
        shown(&self.line, self.names, self.cut, index)
    }

    /// Where reading resumes after a row that consumes the rest of the line: the next
    /// name that directly follows its predecessor, where cscope's display stopped.
    fn after_row(&self, index: usize) -> usize {
        resume_after(self.names, index)
    }
}

/// A unit's stored lines, read one at a time, each line's names into one buffer the lines
/// share rather than a buffer of their own.
struct Lines<'u, L> {
    lines: L,
    names: Vec<Name<'u>>,
}

fn lines(unit: &AnalysisUnit) -> Lines<'_, impl Iterator<Item = Line<'_>>> {
    Lines {
        lines: unit.lines(),
        names: Vec::new(),
    }
}

impl<'u, L: Iterator<Item = Line<'u>>> Lines<'u, L> {
    // The walks call this for every line: kept inline whatever else the build holds, since
    // a call here costs symbol searches about 7%.
    #[inline(always)]
    fn next(&mut self) -> Option<Stored<'u, '_>> {
        let line = self.lines.next()?;
        self.names.clear();
        self.names.extend(line.names());
        let names = &self.names[..];
        let cut = display_cut(names);
        Some(Stored { line, names, cut })
    }
}

/// Fields 0 and 9, sequentially. The context is the macro being defined, else the
/// function, else the file scope. The macro carries from file to file; the function does
/// not. A row consumes the rest of its line, including any context changes there.
fn symbols<'u>(
    matcher: &mut Matcher<'_>,
    files: &[Indexed<'u>],
    assignments: bool,
    rows: &mut Rows<'u>,
) {
    let mut macro_name: Option<&'u [u8]> = None;
    let mut function: Option<&'u [u8]> = None;
    for indexed in files {
        if indexed.first {
            function = None;
        }
        let mut lines = lines(indexed.unit);
        while let Some(stored) = lines.next() {
            let mut index = 0;
            while index < stored.names.len() {
                let name = stored.names[index];
                let matched = match name.mark {
                    Mark::FunctionEnd => {
                        function = None;
                        false
                    }
                    Mark::DefineEnd => {
                        macro_name = None;
                        false
                    }
                    Mark::Include => false,
                    Mark::Function => {
                        function = Some(name.text);
                        matcher.marked(name.text)
                    }
                    Mark::Define => {
                        macro_name = Some(name.text);
                        matcher.marked(name.text)
                    }
                    Mark::Plain => matcher.plain(name.text),
                    _ => matcher.marked(name.text),
                };
                if !matched || (assignments && !name.assigned) {
                    index += 1;
                    continue;
                }
                // Without -C, cscope recognises the macro's own name by where it stored
                // it, so a definition's row takes the enclosing function instead.
                let own_macro = name.mark == Mark::Define && !matcher.caseless;
                let context = match macro_name {
                    Some(name) if !own_macro => Context::Name(name),
                    _ => function.map_or(Context::Global, Context::Name),
                };
                rows.push(Row {
                    path: indexed.path,
                    context,
                    line: stored.line.number,
                    text: stored.text(index),
                });
                index = stored.after_row(index);
            }
            if stored.line.define_end_after {
                macro_name = None;
            }
        }
    }
}

/// Field 1: definitions whose name matches. The context shows the pattern. A row
/// consumes the rest of its line.
fn definitions<'u>(matcher: &mut Matcher<'_>, files: &[Indexed<'u>], rows: &mut Rows<'u>) {
    for indexed in files {
        let mut lines = lines(indexed.unit);
        while let Some(stored) = lines.next() {
            let mut index = 0;
            while index < stored.names.len() {
                let name = stored.names[index];
                let definition = matches!(
                    name.mark,
                    Mark::Define
                        | Mark::Function
                        | Mark::Class
                        | Mark::Enum
                        | Mark::Member
                        | Mark::Struct
                        | Mark::Typedef
                        | Mark::Union
                        | Mark::Global
                );
                if definition && matcher.marked(name.text) {
                    rows.push(Row {
                        path: indexed.path,
                        context: Context::Pattern,
                        line: stored.line.number,
                        text: stored.text(index),
                    });
                    index = stored.after_row(index);
                } else {
                    index += 1;
                }
            }
        }
    }
}

/// Field 8: includes whose operand, without its opening delimiter, matches. The context
/// is the file scope. A row consumes the rest of its line.
fn includers<'u>(matcher: &mut Matcher<'_>, files: &[Indexed<'u>], rows: &mut Rows<'u>) {
    for indexed in files {
        let Some(last_include_line) = indexed.unit.last_include_line() else {
            continue;
        };
        let mut names = Vec::new();
        for line in indexed.unit.lines() {
            if line.number > last_include_line {
                break;
            }
            // Display boundaries are needed only on lines that hold an include.
            if !line.names().any(|name| name.mark == Mark::Include) {
                continue;
            }
            names.clear();
            names.extend(line.names());
            let cut = display_cut(&names);
            let stored = Stored {
                line,
                names: &names,
                cut,
            };
            let mut index = 0;
            while index < stored.names.len() {
                let name = stored.names[index];
                let operand = name.text.get(1..).unwrap_or_default();
                if name.mark == Mark::Include && matcher.marked(operand) {
                    rows.push(Row {
                        path: indexed.path,
                        context: Context::Global,
                        line: stored.line.number,
                        text: stored.text(index),
                    });
                    index = stored.after_row(index);
                } else {
                    index += 1;
                }
            }
        }
    }
}

/// Field 3, sequentially: calls whose name matches, attributed to the macro being defined,
/// else to each distinct function opened since the last function end (at most nine).
/// Calls outside both give no row. The macro and the function list carry from file to
/// file.
fn callers<'u>(matcher: &mut Matcher<'_>, files: &[Indexed<'u>], rows: &mut Rows<'u>) {
    const MOST_CALLERS: usize = 9;
    let mut macro_name: Option<&'u [u8]> = None;
    let mut functions: Vec<&'u [u8]> = Vec::with_capacity(MOST_CALLERS);
    for indexed in files {
        let mut lines = lines(indexed.unit);
        while let Some(stored) = lines.next() {
            for (index, name) in stored.names.iter().enumerate() {
                match name.mark {
                    Mark::Define => macro_name = Some(name.text),
                    Mark::DefineEnd => macro_name = None,
                    Mark::Function => {
                        if functions.len() < MOST_CALLERS && !functions.contains(&name.text) {
                            functions.push(name.text);
                        }
                    }
                    Mark::FunctionEnd => functions.clear(),
                    Mark::Call if matcher.marked(name.text) => {
                        let text = stored.text(index);
                        let line = stored.line.number;
                        match macro_name {
                            Some(name) => rows.push(Row {
                                path: indexed.path,
                                context: Context::Name(name),
                                line,
                                text,
                            }),
                            None => {
                                for function in &functions {
                                    rows.push(Row {
                                        path: indexed.path,
                                        context: Context::Name(function),
                                        line,
                                        text,
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if stored.line.define_end_after {
                macro_name = None;
            }
        }
    }
}

/// Where field 2's reading stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Callees {
    /// Looking for a matching function or macro definition.
    Seeking,
    /// Listing the calls after a matching definition.
    Listing,
    /// Passing over a macro defined inside the listed body.
    Passing,
}

/// Field 2, sequentially: the calls after each matching function or macro definition, up
/// to the function's end, the file's end or, once any macro definition has been seen, a
/// macro's end. Macros defined inside the body are passed over. The context is the called name.
///
/// The end of a file that ends a listing is not seen as a new file, so later rows keep
/// the earlier file's path, as in cscope.
fn callees<'u>(matcher: &mut Matcher<'_>, files: &[Indexed<'u>], rows: &mut Rows<'u>) {
    let mut state = Callees::Seeking;
    let mut macro_seen = false;
    let mut path: &'u [u8] = &[];
    for indexed in files {
        if indexed.first {
            match state {
                Callees::Seeking => path = indexed.path,
                Callees::Listing => state = Callees::Seeking,
                Callees::Passing => {}
            }
        }
        let mut lines = lines(indexed.unit);
        while let Some(stored) = lines.next() {
            for (position, name) in stored.names.iter().enumerate() {
                state = match (state, name.mark) {
                    (Callees::Seeking, Mark::Define) => {
                        macro_seen = true;
                        seek(matcher, name.text)
                    }
                    (Callees::Seeking, Mark::Function) => seek(matcher, name.text),
                    (Callees::Listing, Mark::Call) => {
                        rows.push(Row {
                            path,
                            context: Context::Name(name.text),
                            line: stored.line.number,
                            text: stored.text(position),
                        });
                        Callees::Listing
                    }
                    (Callees::Listing, Mark::Define) => Callees::Passing,
                    (Callees::Listing, Mark::DefineEnd) if macro_seen => Callees::Seeking,
                    (Callees::Listing, Mark::FunctionEnd) => Callees::Seeking,
                    (Callees::Passing, Mark::DefineEnd) => Callees::Listing,
                    (state, _) => state,
                };
            }
            if stored.line.define_end_after {
                state = match state {
                    Callees::Listing if macro_seen => Callees::Seeking,
                    Callees::Passing => Callees::Listing,
                    state => state,
                };
            }
        }
    }
}

fn seek(matcher: &mut Matcher<'_>, name: &[u8]) -> Callees {
    if matcher.marked(name) {
        Callees::Listing
    } else {
        Callees::Seeking
    }
}
