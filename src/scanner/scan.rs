//! The scanner (reading-c.md §2-§6, §10): one pass over the file.
//!
//! The reader works out what each byte begins (a line end, a blank, a literal, a comment,
//! a word, a number, a bracket, punctuation, a directive or a section marker) and
//! collects the line's text as the reference collects it. Names get their role from the
//! look-ahead layer and the declaration state: how braces and parentheses nest, what the
//! current declaration has said, the open conditionals and the region of the file. Each
//! finished line with names becomes a record.

mod conditional_arms;
mod declarators;

use super::assignments;
use super::bodies::{Bodies, TagBody};
use super::input::Input;
use super::keywords::{self, DEFINE_CODE, INCLUDE_CODE, Keyword, Role, keyword_type};
use super::line::Line;
use super::paths;
use super::shapes::{LookAhead, is_blank, is_word, is_word_start};
use super::tails::Tail;
use super::{Corrections, Mode, Record};
use crate::unit::Mark;
use conditional_arms::ConditionalArms;
use declarators::{
    DECLARATOR_WINDOW, Declarators, FinishedName, NameReading, closes_before_suffix, declarator_gap,
};

/// What the reader is inside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    Code,
    /// The word after `#`.
    Directive,
    /// The rest of an `#endif` line, which decides whether the group closes.
    AfterEndif,
    Literal(u8),
    Comment,
}

/// The preceding code tokens, ignoring comments and whitespace across physical lines.
/// A name or type before `(` keeps a pointer declaration from becoming a call.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum CallPrefix {
    #[default]
    Boundary,
    Expression,
    Other,
    Open,
    Dereference,
    DeclarationOpen,
    PointerDeclaration,
}

#[derive(Clone, Copy, Default)]
struct CallContext {
    prefix: CallPrefix,
    /// Closing a control condition or macro parameter list starts an expression.
    closing: Option<i32>,
    cast: Option<i32>,
    /// A written declaration's comma list, outside initializers and array bounds.
    declaration: Option<(i32, i32)>,
    initializer: Option<(i32, i32)>,
    arrays: u32,
}

impl CallContext {
    fn begin_declaration(&mut self, parentheses: i32, braces: i32) {
        if self.declaration.is_none_or(|(_, depth)| depth >= braces) {
            self.declaration = Some((parentheses, braces));
        }
    }
}

/// Brace and parenthesis nesting, and what the open braces belong to.
#[derive(Clone, Default)]
struct Nesting {
    calls: CallContext,
    depth: i32,
    parentheses: i32,
    /// Inside a class body, whose member functions sit one brace deep.
    class_body: bool,
    /// Inside a struct, union or enum body opened at file scope.
    aggregate: bool,
    /// A proved file-scope struct or union body, never an enum or unknown body.
    file_structure: bool,
    /// Inside a function body.
    function: bool,
    /// A named function definition's head has been read and its body has not opened.
    /// A head inside its parentheses is a parameter's type (definition-bodies).
    head_pending: Option<PendingHead>,
    /// Array nesting inside a pending function head. Lex rules and other source
    /// regions keep the reference's independent declaration-subscript state.
    head_subscripts: u32,
    /// An explicit attribute's parentheses and the depth of its labels. Deeper
    /// call-like words are expression payloads, not labels or function heads.
    attribute: Option<Attribute>,
    /// The parenthesis depth outside a proved simple `_Atomic(type)` specifier.
    atomic_type: Option<i32>,
    /// The parenthesis depth at the last `;`, `{` or `}`. The reference's depth can stay
    /// open past a statement (an `#if` arm and an `#else` arm that each open one), so
    /// the parentheses around a head count from here.
    statement_parentheses: i32,
    /// The depths at which open linkage blocks (`extern "C" {`) were opened, with the
    /// linkage-blocks Correction; a linkage block's braces do not nest.
    linkage: Vec<i32>,
    /// The next `{` opens a linkage block.
    linkage_pending: bool,
}

#[derive(Clone, Copy)]
enum PendingHead {
    /// A call-like prefix may still be a wrapper before the written head.
    Provisional,
    /// A declaration prefix establishes the written head.
    Written,
}

#[derive(Clone, Copy)]
struct Attribute {
    outside: i32,
    labels: i32,
}

/// What a brace closed.
enum Closed {
    FunctionBody,
    Other,
}

impl Nesting {
    fn close_brace(&mut self) -> Closed {
        self.depth = self.depth.saturating_sub(1);
        if self.depth <= 0 {
            self.depth = 0;
            self.class_body = false;
        }
        let outermost = self.depth == 0 || (self.depth == 1 && self.class_body);
        if !outermost {
            return Closed::Other;
        }
        if self.aggregate {
            self.aggregate = false;
            self.file_structure = false;
            Closed::Other
        } else if self.function {
            self.function = false;
            Closed::FunctionBody
        } else {
            Closed::Other
        }
    }
}

/// What the current declaration has said so far. Its parts end at `;`, `,`, `)` or `}`
/// as each allows.
#[derive(Clone, Default)]
struct Declaration {
    /// The brace depth of an `=` being read, until `,`, `;` or its closing brace.
    assignment: Option<i32>,
    /// After `:`, a bit-field width, until `,` or `;`.
    width: bool,
    /// Inside `[` `]`.
    subscript: bool,
    /// The brace depth of a `typedef`, until `;` there.
    typedef: Option<i32>,
    /// A `typedef` still expects its type word.
    type_expected: bool,
    /// A keyword that a pending `typedef` took for its type was read as a call head
    /// (`void (` in `typedef void (*f)();`), and its form ended without counting as the
    /// type. With the parenthesised-typedefs Correction the next name decides: when it
    /// is the name of the group after the keyword, the keyword was the type.
    type_form: bool,
    /// With the typedef-qualifiers Correction, a qualifier right after `typedef` left
    /// the type pending: the next name at the typedef's depth is the type's (a plain
    /// name), or the declarator's where `;`, `,`, `[` or `)` follows it (an implicit
    /// `int`); a body opened at the typedef's depth is the type.
    qualified: bool,
    /// The brace depth of an `extern`, until `;` there.
    extern_depth: Option<i32>,
    declarators: Declarators,
    /// The tag letter the next name takes, after `struct`, `union`, `enum` or `class`.
    tag: Option<u8>,
    /// With the definition-bodies Correction, a word was read at the declaration's own
    /// parenthesis level since its `;`, `{` or `}`, or the declaration goes on after the
    /// body of an aggregate it declares: a head without a body read now does not begin it.
    begun: bool,
    /// A written type word at the declaration's own level, possibly on a previous
    /// line. Storage classes alone cannot establish a function head's return type.
    type_prefix: bool,
    /// Whether the typedef's prefix still consists of words or a completed
    /// aggregate, and whether this declarator already has a pointer star.
    typedef_prefix: TypedefPrefix,
    /// Written tokens since the last declaration boundary, for opaque tails.
    tail: Tail,
}

#[derive(Clone, Copy, Default)]
struct TypedefPrefix {
    words: bool,
    pointer: bool,
}

impl Declaration {
    fn at_typedef_prefix(&self, nesting: &Nesting) -> bool {
        self.typedef == Some(nesting.depth)
            && nesting.parentheses == nesting.statement_parentheses
            && !self.subscript
    }

    /// Nothing in the declaration but a form it may be in says that a name is anything
    /// but declared here: no initializer, `extern`, bit-field width or subscript.
    fn declaring(&self) -> bool {
        self.assignment.is_none() && self.extern_depth.is_none() && !self.width && !self.subscript
    }

    fn keyword(&mut self, role: Role, depth: i32, parentheses: i32, qualifier: bool) {
        Declarators::keyword(self, role, depth, parentheses, qualifier);
    }

    /// The mark a name gets, given the mark its shape chose; and whether it names an
    /// aggregate's tag (which opens the aggregate at file scope).
    fn name(&mut self, word: &[u8], shaped: Mark, depth: i32) -> (Mark, bool) {
        if let Some(tag) = self.tag
            && word != b"class"
        {
            self.tag = None;
            return (Mark::tag(tag).unwrap_or(shaped), true);
        }
        if self.typedef == Some(depth) && !self.type_expected && !self.subscript {
            return (Mark::Typedef, false);
        }
        self.type_expected = false;
        (shaped, false)
    }

    fn end_statement(&mut self, depth: i32) {
        if self.typedef == Some(depth) {
            self.typedef = None;
            self.declarators.reset_typedef();
            self.type_form = false;
            self.qualified = false;
            self.typedef_prefix = TypedefPrefix::default();
        }
        if self.extern_depth == Some(depth) {
            self.extern_depth = None;
        }
        self.assignment = None;
        self.width = false;
    }
}

/// Where in the file the reader is.
struct Region {
    /// Declarations here are at file scope; lex and yacc files start outside it.
    file_scope: bool,
    /// Between the `%%` markers of a lex or yacc file.
    rules_section: bool,
    /// Inside a macro definition's body.
    macro_body: bool,
    /// Inside a directive's logical line, continuation lines included.
    directive: bool,
    /// The expression of a line-initial `#if` or `#elif`, outside macro replacement text.
    conditional_expression: bool,
}

/// Where the initial replacement text starts, after any macro parameters.
enum MacroStart {
    /// Only adjacent splices may still put a formal list after the name.
    Header(usize),
    Parameters(i32),
    Text(usize),
}

pub(super) struct Scan<'s> {
    input: Input<'s>,
    look: LookAhead<'s>,
    mode: Mode,
    corrections: Corrections,
    /// The Corrections' look-aheads, which share one work limit.
    bodies: Bodies,
    /// With the macro-bodies Correction, the nesting and declaration state from before
    /// the macro definition being read.
    before_macro: Option<(Nesting, Declaration)>,
    macro_start: Option<MacroStart>,
    reading: Reading,
    /// The last byte consumed was a line end.
    at_line_start: bool,
    /// The physical line being read, and the line the current record started on.
    line: u32,
    record_line: u32,
    collected: Line,
    records: Vec<Record>,
    nesting: Nesting,
    declaration: Declaration,
    conditionals: ConditionalArms,
    region: Region,
}

/// Whether a name may be promoted to a definition head (definition-heads): not a
/// keyword, not an attribute, and not right after `struct`, `union` or `enum`, where it
/// is a tag's place (`struct __attribute__((packed)) { ... }`). `before` is the line's
/// text before the name.
/// Marks the identifier of a function head `MACRO(name)(` as a Function too, where the
/// head's macro takes only that identifier and the function's own list follows on the
/// same line (written-head-names).
fn written_head_names(record: &mut Record) {
    let text = &record.text;
    let blanks = |at: usize| {
        at + text[at..]
            .iter()
            .take_while(|byte| is_blank(**byte))
            .count()
    };
    for index in 1..record.symbols.len() {
        let (head, name) = (record.symbols[index - 1], record.symbols[index]);
        if head.mark != Mark::Function || name.mark != Mark::Plain {
            continue;
        }
        let between = blanks(head.at + head.len);
        if text.get(between) != Some(&b'(') || blanks(between + 1) != name.at {
            continue;
        }
        let close = blanks(name.at + name.len);
        if text.get(close) != Some(&b')') || text.get(blanks(close + 1)) != Some(&b'(') {
            continue;
        }
        if keywords::keyword(&text[name.at..name.at + name.len]).is_none() {
            record.symbols[index].mark = Mark::Function;
        }
    }
}

fn may_head(before: &[u8], name: &[u8]) -> bool {
    let end = before
        .iter()
        .rposition(|byte| !is_blank(*byte))
        .map_or(0, |at| at + 1);
    let start = before[..end]
        .iter()
        .rposition(|byte| !is_word(*byte))
        .map_or(0, |at| at + 1);
    !matches!(&before[start..end], b"struct" | b"union" | b"enum")
        && keywords::keyword(name).is_none()
        && !is_attribute(name)
}

/// Whether `name` spells an attribute (`__attribute__`, `__declspec`), which is never a
/// function.
fn is_attribute(name: &[u8]) -> bool {
    matches!(name, b"__attribute__" | b"__attribute" | b"__declspec")
}

/// A declaration word immediately precedes this head on its line. A closing
/// parameter list is not a prefix for a trailing annotation. The small window
/// leaves long or split prefixes to the existing declaration state.
fn head_prefix(before: &[u8]) -> bool {
    let before = &before[before.len().saturating_sub(DECLARATOR_WINDOW)..];
    let Some(end) = before.iter().rposition(|byte| !is_blank(*byte)) else {
        return false;
    };
    if before[end] == b'*' {
        return true;
    }
    if !is_word(before[end]) {
        return false;
    }
    let start = before[..=end]
        .iter()
        .rposition(|byte| !is_word(*byte))
        .map_or(0, |at| at + 1);
    let word = &before[start..=end];
    keyword_type(word) || (keywords::keyword(word).is_none() && word != b"inline")
}

/// A directive tail made entirely of blanks and closed block comments, at least
/// one crossing a line. A single bounded scan proves the whole closing line is
/// still directive trivia before any input is consumed.
fn closed_conditional_tail(input: &Input, mut at: usize) -> Option<usize> {
    const WINDOW: usize = 64 * 1024;
    let mut multiline = false;
    while at < WINDOW {
        at += input.run_within(at, WINDOW, is_blank);
        if at >= WINDOW {
            return None;
        }
        match (input.at(at), input.at(at + 1)) {
            (None | Some(b'\n'), _) => return multiline.then_some(at),
            (Some(b'/'), Some(b'*')) => {
                at += 2;
                loop {
                    if at + 1 >= WINDOW {
                        return None;
                    }
                    match (input.at(at)?, input.at(at + 1)) {
                        (b'\\', Some(b'\n')) => return None,
                        (b'\\', Some(b'\r')) if input.at(at + 2) == Some(b'\n') => return None,
                        (b'*', Some(b'/')) => {
                            at += 2;
                            break;
                        }
                        (b'\n', _) => multiline = true,
                        _ => {}
                    }
                    at += 1;
                }
            }
            _ => return None,
        }
    }
    None
}

/// What text after a name makes of it.
#[derive(Clone, Copy)]
enum After {
    Definition,
    Call,
    Declarator,
    Other,
}

/// What text after `struct`, `union` or `enum` makes of it.
#[derive(Clone, Copy)]
enum AfterTagKeyword {
    Definition,
    Body,
    Use,
    Other,
}

/// Where an include operand that follows `#include` (ending at `from`) ends: blanks, then
/// `"`...`"` or `<`...`>` on one line, not empty.
fn include_operand(input: &Input, from: usize) -> Option<usize> {
    let open = from + input.run(from, is_blank);
    let closer = match input.at(open)? {
        b'"' => b'"',
        b'<' => b'>',
        _ => return None,
    };
    let inside = input.run(open + 1, |byte| byte != closer && byte != b'\n');
    (inside > 0 && input.at(open + 1 + inside) == Some(closer)).then_some(open + inside + 2)
}

impl<'s> Scan<'s> {
    pub(super) fn new(bytes: &'s [u8], mode: Mode, corrections: Corrections) -> Self {
        Self {
            input: Input::new(bytes),
            look: LookAhead::new(bytes),
            mode,
            corrections,
            bodies: Bodies::new(corrections.conditional_lists),
            before_macro: None,
            macro_start: None,
            reading: Reading::Code,
            at_line_start: true,
            line: 1,
            record_line: 1,
            collected: Line::default(),
            records: Vec::new(),
            nesting: Nesting::default(),
            declaration: Declaration::default(),
            conditionals: ConditionalArms::new(corrections),
            region: Region {
                file_scope: !matches!(mode, Mode::Lex | Mode::Yacc),
                rules_section: false,
                macro_body: false,
                directive: false,
                conditional_expression: false,
            },
        }
    }

    pub(super) fn run(mut self) -> Vec<Record> {
        self.read_all();
        self.records
    }

    #[cfg(test)]
    pub(super) fn run_with_cost(mut self, unbounded: bool) -> (Vec<Record>, super::ScanCost) {
        if unbounded {
            self.bodies.unbounded();
            self.conditionals.unbounded();
        }
        self.read_all();
        let mut cost = self.bodies.cost();
        cost.closing_refusals = self.conditionals.refusals;
        (self.records, cost)
    }

    fn read_all(&mut self) {
        while let Some(byte) = self.input.at(0) {
            match self.reading {
                Reading::Code => self.code(byte),
                Reading::Directive => self.directive(byte),
                Reading::AfterEndif => self.after_endif(),
                Reading::Literal(closer) => self.literal(byte, closer),
                Reading::Comment => self.comment(byte),
            }
        }
        if let Some(record) = self.collected.finish(self.record_line) {
            self.push_record(record);
        }
    }

    /// Keeps a finished line's record, with the written name of a macro-named head
    /// defined too (written-head-names).
    fn push_record(&mut self, mut record: Record) {
        if self.corrections.written_head_names {
            written_head_names(&mut record);
        }
        self.records.push(record);
    }

    // Consuming input.

    /// Consumes `length` bytes into the line's text.
    fn take(&mut self, length: usize) {
        let bytes = self.input.peek(length);
        self.input.advance(length);
        self.at_line_start = bytes.last() == Some(&b'\n');
        self.collected.add(&bytes);
    }

    /// Consumes `length` bytes without collecting them.
    fn skip(&mut self, length: usize) {
        let last = self.input.at(length.saturating_sub(1));
        self.input.advance(length);
        self.at_line_start = length > 0 && last == Some(b'\n');
    }

    /// Consumes one byte, writing `bytes` in place of a marker.
    fn put(&mut self, bytes: &[u8]) {
        self.input.advance(1);
        self.at_line_start = false;
        self.collected.put(bytes);
    }

    /// A line ends. Unless a backslash continued it, so does any directive on it.
    fn next_line(&mut self) {
        self.region.directive = false;
        self.region.conditional_expression = false;
        self.continue_line();
    }

    /// A line ends with a backslash before its line end.
    fn continue_line(&mut self) {
        self.line = self.line.saturating_add(1);
        if let Some(record) = self.collected.finish(self.record_line) {
            self.push_record(record);
        }
        self.record_line = self.line;
    }

    // Code.

    fn code(&mut self, byte: u8) {
        let next = self.input.at(1);
        if let Some(MacroStart::Header(from)) = self.macro_start {
            self.macro_start = if self.bodies.macro_header_splice(&self.input) {
                Some(MacroStart::Header(from))
            } else if byte == b'(' {
                Some(MacroStart::Parameters(self.nesting.parentheses))
            } else {
                Some(MacroStart::Text(from))
            };
        }
        if !is_word_start(byte)
            && (!self.in_directive() || self.region.macro_body)
            && !is_blank(byte)
            && !matches!(byte, b'\n' | b'#')
            && !(byte == b'/' && matches!(next, Some(b'*' | b'/')))
            && !(byte == b'\\' && matches!(next, Some(b'\n' | b'\r')))
        {
            self.nesting.calls.prefix = match byte {
                b'(' if !matches!(self.nesting.calls.prefix, CallPrefix::Other) => CallPrefix::Open,
                b'(' => CallPrefix::DeclarationOpen,
                b'*' if matches!(self.nesting.calls.prefix, CallPrefix::Open) => {
                    CallPrefix::Dereference
                }
                b'*' if matches!(self.nesting.calls.prefix, CallPrefix::Other) => CallPrefix::Other,
                b'*' if matches!(
                    self.nesting.calls.prefix,
                    CallPrefix::DeclarationOpen | CallPrefix::PointerDeclaration
                ) =>
                {
                    CallPrefix::PointerDeclaration
                }
                b')' if self.nesting.calls.closing == Some(self.nesting.parentheses - 1)
                    || self.nesting.calls.cast == Some(self.nesting.parentheses - 1) =>
                {
                    if self.nesting.calls.closing == Some(self.nesting.parentheses - 1) {
                        self.nesting.calls.closing = None;
                    }
                    if self.nesting.calls.cast == Some(self.nesting.parentheses - 1) {
                        self.nesting.calls.cast = None;
                    }
                    CallPrefix::Expression
                }
                b')' | b']' | b'"' | b'\'' | b'#' | b'0'..=b'9' => CallPrefix::Other,
                b';' | b'{' | b'}' => CallPrefix::Boundary,
                _ => CallPrefix::Expression,
            };
        }
        match byte {
            b'\n' => self.line_end(),
            b'\\' if next == Some(b'\n') => {
                self.block_typedef_prefix();
                self.declaration.tail.block();
                self.skip(2);
                self.collected.add(b"\\");
                self.continue_line();
            }
            b'\\'
                if self.corrections.crlf_continuations
                    && next == Some(b'\r')
                    && self.input.at(2) == Some(b'\n') =>
            {
                self.block_typedef_prefix();
                self.declaration.tail.block();
                self.skip(3);
                self.collected.add(b"\\");
                self.continue_line();
            }
            _ if is_blank(byte) => self.blanks(),
            b'/' if next == Some(b'*') => {
                self.skip(2);
                self.reading = Reading::Comment;
            }
            b'/' if next == Some(b'/') => {
                let rest = self.input.run(2, |byte| byte != b'\n');
                if self.comment_ends_macro(0) {
                    // The line end is read on its own, so it ends the macro.
                    self.skip(2 + rest);
                    return;
                }
                let line_end = usize::from(self.input.at(2 + rest).is_some());
                self.skip(2 + rest + line_end);
                self.next_line();
            }
            b'"' => self.open_literal(b'"'),
            b'\'' if self.mode != Mode::StateDescription => self.open_literal(b'\''),
            _ if is_word_start(byte) => self.word(),
            _ if byte.is_ascii_digit()
                || (byte == b'.' && next.is_some_and(|n| n.is_ascii_digit())) =>
            {
                self.number()
            }
            b'#' => self.hash(),
            b'*' => {
                if self.at_typedef_prefix() {
                    self.declaration.typedef_prefix.pointer = true;
                }
                self.take(1);
            }
            b'%' if next == Some(b'{') => {
                self.take(2);
                self.region.file_scope = true;
            }
            b'%' if next == Some(b'}') => {
                self.take(2);
                self.region.file_scope = false;
            }
            b'%' if next == Some(b'%') && self.at_line_start => self.rules_marker(),
            b'{' | b'}' | b'(' | b')' | b'[' | b']' => self.bracket(byte),
            b'=' | b':' | b',' | b';' => self.punctuation(byte),
            _ => {
                self.block_typedef_prefix();
                self.take(1);
            }
        }
    }

    /// With the macro-line-comments Correction, a line comment starts at `at` in a macro
    /// body and ends the macro with its line: no backslash (before any CR) at its end
    /// continues it onto the next line, as C would.
    fn comment_ends_macro(&self, at: usize) -> bool {
        if !(self.corrections.macro_line_comments
            && self.region.macro_body
            && self.input.at(at) == Some(b'/')
            && self.input.at(at + 1) == Some(b'/'))
        {
            return false;
        }
        let end = at + 2 + self.input.run(at + 2, |byte| byte != b'\n');
        let before = |back: usize| self.input.at(end - back);
        !(before(1) == Some(b'\\') || (before(1) == Some(b'\r') && before(2) == Some(b'\\')))
    }

    fn line_end(&mut self) {
        if self.region.macro_body {
            // The macro ends here, and the line end is read again outside it.
            self.region.macro_body = false;
            self.macro_start = None;
            if let Some((nesting, declaration)) = self.before_macro.take() {
                self.nesting = nesting;
                self.declaration = declaration;
            }
            self.collected.marker(Mark::DefineEnd);
            return;
        }
        self.skip(1);
        self.next_line();
    }

    /// A run of blanks: dropped at the start or end of a line, kept as one blank
    /// otherwise, with a single space kept as it is.
    fn blanks(&mut self) {
        let run = self.input.run(0, is_blank);
        let only_spaces_single = run == 1 && self.input.at(0) == Some(b' ');
        if self.input.at(run) == Some(b'\n') || self.at_line_start || self.comment_ends_macro(run) {
            self.skip(run);
        } else if only_spaces_single {
            self.take(1);
        } else {
            self.skip(run);
            self.input.push_back(b" ");
        }
    }

    fn open_literal(&mut self, quote: u8) {
        self.block_typedef_prefix();
        self.declaration.tail.block();
        self.take(1);
        self.reading = Reading::Literal(quote);
    }

    fn number(&mut self) {
        self.block_typedef_prefix();
        if self.nesting.parentheses == 0 && !self.region.directive {
            self.declaration.tail.block();
        }
        let rest = self.input.run(1, |byte| {
            byte.is_ascii_digit()
                || byte == b'.'
                || matches!(byte, b'x' | b'X' | b'a'..=b'f' | b'A'..=b'F' | b'u' | b'U' | b'l' | b'L')
        });
        self.take(1 + rest);
    }

    /// `#`: blanks after it are dropped, and outside a rules section a directive follows.
    fn hash(&mut self) {
        let blanks = self.input.run(1, is_blank);
        self.skip(1 + blanks);
        self.collected.add(b"#");
        if !self.region.rules_section {
            self.reading = Reading::Directive;
            self.region.directive = true;
            self.region.conditional_expression = false;
        }
    }

    fn bracket(&mut self, byte: u8) {
        if byte == b'(' && self.at_typedef_prefix() && self.nesting.attribute.is_none() {
            self.declaration.typedef_prefix.words = false;
        }
        if self.tracks_tails() && !self.region.directive && !self.region.macro_body {
            match byte {
                b'{' => self.declaration.tail = Tail::default(),
                b'}' if self.nesting.function
                    && !self.nesting.aggregate
                    && self.nesting.depth == 1 =>
                {
                    self.declaration.tail = Tail::default();
                }
                b'}' | b'[' | b']' => self.declaration.tail.block(),
                b'(' if self.nesting.parentheses == 0 => self.declaration.tail.invocation(),
                _ => {}
            }
        }
        if matches!(byte, b'{' | b'}') {
            if byte == b'}'
                && self
                    .nesting
                    .calls
                    .declaration
                    .is_some_and(|(_, depth)| depth >= self.nesting.depth)
            {
                self.nesting.calls.declaration = None;
            }
            self.nesting.statement_parentheses = self.nesting.parentheses;
            self.declaration.begun = false;
            self.declaration.type_prefix = false;
        }
        match byte {
            b'{' if self.nesting.linkage_pending => {
                self.nesting.linkage_pending = false;
                self.nesting.linkage.push(self.nesting.depth);
            }
            b'}' if self.nesting.linkage.last() == Some(&self.nesting.depth) => {
                self.nesting.linkage.pop();
            }
            b'{' => {
                self.declaration
                    .declarators
                    .before_body(self.nesting.depth, self.nesting.parentheses);
                // With the typedef-qualifiers Correction, a body after the qualifier is the
                // typedef's type (`typedef const struct tag {} name;`).
                if self.declaration.qualified
                    && self.declaration.typedef == Some(self.nesting.depth)
                {
                    self.declaration.qualified = false;
                    self.declaration.type_expected = false;
                }
                self.nesting.depth = self.nesting.depth.saturating_add(1);
                self.nesting.head_pending = None;
                self.declaration.declarators.begin_body();
                if let Some(tag) = self.declaration.tag.take() {
                    // A tag's body opens: the brace stands in for an empty-named marker.
                    if self.nesting.depth == 1 {
                        self.nesting.aggregate = true;
                    }
                    let at = self.collected.anchor;
                    self.put(b"{");
                    if let Some(mark) = Mark::tag(tag) {
                        self.collected.name(mark, at, at);
                    }
                    return;
                }
            }
            b'}' => {
                self.declaration
                    .declarators
                    .end_declarator_list(self.nesting.depth);
                if self.declaration.assignment == Some(self.nesting.depth) {
                    self.declaration.assignment = None;
                }
                let aggregate = self.nesting.aggregate;
                let mut closed = self.nesting.close_brace();
                // With alternative-bodies, a body followed by another arm's body under the
                // same head leaves the function open for it.
                if let Closed::FunctionBody = closed
                    && self.corrections.alternative_bodies
                    && self.conditionals.after_head(self.nesting.depth)
                    && conditional_arms::alternative_body(&self.input)
                {
                    self.nesting.function = true;
                    closed = Closed::Other;
                }
                // A declaration goes on after the body of an aggregate it declares
                // (`struct s { ... } __packed;`).
                self.declaration.begun = aggregate && !self.nesting.aggregate;
                self.declaration.type_prefix = false;
                if let Closed::FunctionBody = closed {
                    // With the extern-definitions Correction an `extern` pending at the
                    // function's depth, normally the one before its head, ends with its
                    // body.
                    if self.corrections.extern_definitions
                        && self.declaration.extern_depth == Some(self.nesting.depth)
                    {
                        self.declaration.extern_depth = None;
                    }
                    let at = self.collected.anchor;
                    self.put(b"}");
                    self.collected.name(Mark::FunctionEnd, at, at);
                    return;
                }
            }
            b'(' => {
                self.declaration.declarators.open_list(
                    &self.nesting,
                    &self.region,
                    &self.input,
                    self.collected.text(),
                    self.corrections,
                );
                self.nesting.parentheses = self.nesting.parentheses.saturating_add(1);
            }
            b')' => {
                let inside = self.nesting.parentheses;
                self.nesting.parentheses = self.nesting.parentheses.saturating_sub(1).max(0);
                if matches!(self.macro_start, Some(MacroStart::Parameters(outside)) if self.nesting.parentheses == outside)
                {
                    self.macro_start = self.input.source_position(1).map(MacroStart::Text);
                }
                if self.nesting.atomic_type == Some(self.nesting.parentheses) {
                    self.nesting.atomic_type = None;
                }
                if self
                    .nesting
                    .attribute
                    .is_some_and(|attribute| self.nesting.parentheses <= attribute.outside)
                {
                    self.nesting.attribute = None;
                }
                self.declaration
                    .declarators
                    .close_list(inside, self.nesting.parentheses);
            }
            b'[' => {
                self.nesting.calls.arrays = self.nesting.calls.arrays.saturating_add(1);
                self.declaration.subscript = true;
                if self.nesting.head_pending.is_some() {
                    self.nesting.head_subscripts = self.nesting.head_subscripts.saturating_add(1);
                }
            }
            _ => {
                self.nesting.calls.arrays = self.nesting.calls.arrays.saturating_sub(1);
                self.declaration.subscript = false;
                self.nesting.head_subscripts = self.nesting.head_subscripts.saturating_sub(1);
            }
        }
        self.take(1);
    }

    fn punctuation(&mut self, byte: u8) {
        let here = (self.nesting.parentheses, self.nesting.depth);
        if byte == b'='
            && self.nesting.calls.declaration == Some(here)
            && self.nesting.calls.initializer.is_none()
        {
            self.nesting.calls.initializer = Some(here);
        } else if byte == b',' && self.nesting.calls.initializer == Some(here) {
            self.nesting.calls.initializer = None;
        }
        if matches!(byte, b'=' | b':') {
            self.block_typedef_prefix();
        }
        let declaring = self.region.file_scope && !self.region.macro_body && !self.in_directive();
        if self.tracks_tails() && declaring {
            if byte == b';' && self.nesting.parentheses == 0 {
                self.declaration.tail = Tail::default();
            } else if self.nesting.parentheses == 0 || matches!(byte, b';' | b'*' | b':') {
                self.declaration.tail.block();
            }
        }
        match byte {
            // With the initializer-entries Correction an `=` inside an initializer (a
            // designated entry) leaves the initializer open.
            b'=' if declaring
                && !(self.corrections.initializer_entries
                    && self.declaration.assignment.is_some()) =>
            {
                self.declaration.assignment = Some(self.nesting.depth);
            }
            b':' if declaring => self.declaration.width = true,
            b',' => {
                if self.at_typedef_prefix() {
                    self.declaration.typedef_prefix.pointer = false;
                }
                if self.declaration.assignment == Some(self.nesting.depth) {
                    self.declaration.assignment = None;
                }
                self.declaration
                    .declarators
                    .end_declared_function(self.nesting.parentheses);
                self.declaration.width = false;
            }
            b';' => {
                if self
                    .nesting
                    .calls
                    .declaration
                    .is_none_or(|(_, depth)| depth >= self.nesting.depth)
                {
                    self.nesting.calls.declaration = None;
                    self.nesting.calls.initializer = None;
                }
                self.declaration
                    .declarators
                    .end_declarator_list(self.nesting.depth);
                self.nesting.statement_parentheses = self.nesting.parentheses;
                self.declaration.begun = false;
                self.declaration.type_prefix = false;
                if self.nesting.depth == 0 {
                    self.nesting.aggregate = false;
                }
                self.declaration
                    .declarators
                    .end_declared_function(self.nesting.parentheses);
                self.declaration.end_statement(self.nesting.depth);
            }
            _ => {}
        }
        self.take(1);
    }

    /// A line-initial `%%` in a lex or yacc file: the rules section opens as a function
    /// named for its scanner or parser, and closes again at the next one.
    fn rules_marker(&mut self) {
        self.skip(2);
        self.nesting.depth = 0;
        if self.region.rules_section {
            self.region.rules_section = false;
            self.region.file_scope = true;
            let at = self.collected.anchor;
            self.collected.put(b"%%");
            self.collected.name(Mark::FunctionEnd, at, at);
            return;
        }
        self.region.rules_section = true;
        // The section reads as a function named for what the file generates; a parser
        // (anything but a lex file) also calls the scanner.
        let (name, call): (&[u8], Option<&[u8]>) = match self.mode {
            Mode::Lex => (b"yylex", None),
            _ => (b"yyparse", Some(b" yylex()")),
        };
        self.collected.put(b"%% /* ");
        let start = self.collected.len();
        if let Some(call) = call {
            self.input.push_back(call);
        }
        self.collected.put(name);
        let end = self.collected.len();
        self.collected.put(b" */");
        self.collected.anchor = start;
        self.collected.name(Mark::Function, start, end);
    }

    // Words.

    fn word(&mut self) {
        let length = self.input.run(0, is_word);
        let exactly = |keyword: &[u8]| length == keyword.len() && self.input.starts_with(keyword);
        let prefix = self.nesting.calls.prefix;
        let code = !self.in_directive() || self.region.macro_body;
        let declaration_word = [
            b"void".as_slice(),
            b"char",
            b"short",
            b"int",
            b"long",
            b"float",
            b"double",
            b"signed",
            b"unsigned",
            b"struct",
            b"union",
            b"enum",
            b"const",
            b"volatile",
            b"auto",
            b"register",
            b"static",
            b"extern",
            b"typedef",
            b"_Bool",
            b"_Complex",
            b"_Atomic",
            b"_Alignas",
            b"_Thread_local",
        ]
        .iter()
        .any(|word| exactly(word));
        if code
            && (self.corrections.dereferenced_calls || self.corrections.split_calls)
            && declaration_word
            && self.calls_are_recorded()
        {
            if matches!(prefix, CallPrefix::Boundary) && !self.region.macro_body {
                self.nesting
                    .calls
                    .begin_declaration(self.nesting.parentheses, self.nesting.depth);
            } else if matches!(prefix, CallPrefix::Open | CallPrefix::DeclarationOpen) {
                let depth = self.nesting.parentheses - 1;
                if self.nesting.calls.closing == Some(depth) && !self.region.macro_body {
                    self.nesting
                        .calls
                        .begin_declaration(self.nesting.parentheses, self.nesting.depth);
                } else {
                    self.nesting.calls.cast = Some(depth);
                }
            }
        }
        if code && (exactly(b"if") || exactly(b"for") || exactly(b"while") || exactly(b"switch")) {
            self.nesting.calls.closing = Some(self.nesting.parentheses);
        }
        self.nesting.calls.prefix = if !code {
            prefix
        } else if matches!(prefix, CallPrefix::PointerDeclaration)
            && (exactly(b"const") || exactly(b"volatile") || exactly(b"restrict"))
        {
            CallPrefix::PointerDeclaration
        } else if exactly(b"return") || exactly(b"else") || exactly(b"do") {
            CallPrefix::Expression
        } else {
            CallPrefix::Other
        };
        let tag_keyword = exactly(b"enum") || exactly(b"struct") || exactly(b"union");
        if self.atomic_type_word(length) {
            return;
        }
        if self
            .look
            .class_definition(&self.input)
            .is_some_and(|reach| reach >= length)
        {
            self.nesting.class_body = true;
            self.declaration.tag = Some(b'c');
            self.take(5);
            return;
        }
        if tag_keyword {
            let start = self.collected.anchor;
            self.take(length);
            self.after_tag_keyword(start);
            return;
        }
        if self
            .look
            .if_parenthesis(&self.input)
            .is_some_and(|reach| reach > length)
        {
            // `if` right before `(` is kept as written, never looked up [09-display].
            self.take(2);
            return;
        }
        let start = self.collected.anchor;
        self.take(length);
        self.after_name(start, prefix);
    }

    /// The type operand is already proved complete within the declarator window.
    /// Render it normally, without letting its words change declaration state.
    fn atomic_type_word(&mut self, length: usize) -> bool {
        if !self.corrections.atomic_types || self.in_directive() || self.region.macro_body {
            return false;
        }
        let introducer = self.nesting.atomic_type.is_none()
            && length == b"_Atomic".len()
            && self.input.starts_with(b"_Atomic")
            && simple_atomic_type(&self.input, length);
        if !introducer && self.nesting.atomic_type.is_none() {
            return false;
        }
        if introducer {
            self.nesting.atomic_type = Some(self.nesting.parentheses);
            self.declaration.type_expected = false;
            self.declaration.type_form = false;
            self.declaration.qualified = false;
            self.begins_declaration(true);
        }
        let start = self.collected.anchor;
        self.take(length);
        if let Some(keyword) = keywords::keyword(&self.collected.text()[start..]) {
            if let Some(code) = keyword.code {
                self.collected.write_code(start, code);
            }
        } else {
            self.collected
                .name(Mark::Plain, start, self.collected.len());
        }
        true
    }

    /// Gives the name that ends at the read position its role from the text after it
    /// (reading-c.md §5).
    fn after_name(&mut self, start: usize, call_prefix: CallPrefix) {
        if self.tracks_tails()
            && self.region.file_scope
            && !self.nesting.function
            && !self.region.macro_body
            && !self.in_directive()
        {
            let keyword = super::tails::declaration_word(&self.collected.text()[start..]);
            if self.nesting.parentheses == 0 {
                self.declaration.tail.word(keyword);
            } else {
                // A word inside a preceding invocation could be a type operand,
                // as in `TYPEOF(T) member`. Numeric padding calls need no names.
                self.declaration.tail.block();
            }
        }
        if self.attribute_name(start) {
            return;
        }
        if self.corrections.dereferenced_calls
            && self.calls_are_recorded()
            && (!self.in_directive() || self.region.macro_body)
        {
            let group = self.nesting.parentheses - 1;
            let declaration =
                self.nesting
                    .calls
                    .declaration
                    .is_some_and(|(parentheses, braces)| {
                        parentheses <= group && braces <= self.nesting.depth
                    })
                    && self.nesting.calls.initializer.is_none()
                    && self.nesting.calls.arrays == 0;
            if matches!(call_prefix, CallPrefix::PointerDeclaration)
                && closes_before_suffix(&self.input)
            {
                self.nesting
                    .calls
                    .begin_declaration(group, self.nesting.depth);
            }
            if matches!(call_prefix, CallPrefix::Dereference)
                && !declaration
                && dereferenced_suffix(&self.input)
            {
                self.found(self.call_mark(start), start, self.collected.len());
                return;
            }
        }
        if self.corrections.split_calls
            && self.nesting.function
            && self.nesting.head_pending.is_none()
            && !self.region.macro_body
            && !self.in_directive()
            && (self.nesting.calls.declaration.is_none()
                || self.nesting.calls.initializer.is_some()
                || self.nesting.calls.arrays > 0)
            && keywords::keyword(&self.collected.text()[start..]).is_none()
            && split_call_suffix(&self.input)
        {
            self.found(self.call_mark(start), start, self.collected.len());
            return;
        }
        let candidates = [
            (After::Definition, self.look.definition(&self.input)),
            (After::Call, self.look.call(&self.input)),
            (After::Declarator, self.look.declarator(&self.input)),
            (After::Other, self.input.at(0).map(|_| 1)),
        ];
        // The longest reach wins; on a tie, the earlier shape.
        let Some((after, _)) = candidates
            .into_iter()
            .filter_map(|(after, reach)| Some((after, reach?)))
            .reduce(|best, candidate| {
                if candidate.1 > best.1 {
                    candidate
                } else {
                    best
                }
            })
        else {
            // The file ends right after the name: the reference never decides its role.
            return;
        };
        // With definition-bodies an attribute is a specifier, even before a body. Other
        // heads without a body become declaration forms; directive heads stay calls.
        // A keyword there is only a keyword.
        let mut declared_function = false;
        let mut declared_keyword = false;
        let mut numeric_specifier = false;
        // The parentheses around the name, counted within its statement.
        let inside = (self.nesting.parentheses - self.nesting.statement_parentheses).max(0);
        let parameter_type = self.nesting.head_pending.is_some() && inside > 0;
        // Prefix evidence is used only where a function can be defined. Most names
        // occur inside bodies, so they need no backwards declaration scan.
        let can_define = self.may_define_function();
        let prefix = can_define
            && (self.declaration.type_prefix || head_prefix(&self.collected.text()[..start]));
        let trailing_annotation = matches!(self.nesting.head_pending, Some(PendingHead::Written))
            && inside == 0
            && !prefix;
        let return_type_wrapper = self.corrections.return_type_wrappers
            && matches!(after, After::Definition | After::Call)
            && can_define
            && inside == 0
            && !self.nesting.function
            && !prefix
            && keywords::keyword(&self.collected.text()[start..]).is_none()
            && self.bodies.return_type_wrapper(&self.input);
        // With the definition-heads Correction a name whose list the definition shape
        // rejects is a definition head when a body follows the list directly, unless a
        // definition is already open (`__releases(x)` between a head and its body).
        let promoted = matches!(after, After::Call | After::Other)
            && self.corrections.definition_heads
            && (!self.nesting.function
                || (prefix
                    && matches!(self.nesting.head_pending, Some(PendingHead::Provisional))
                    && head_prefix(&self.collected.text()[..start])
                    && self.bodies.typed_parameters(&self.input)))
            && inside == 0
            && !self.region.directive
            && can_define
            && may_head(
                &self.collected.text()[..start],
                &self.collected.text()[start..],
            )
            && self.bodies.direct(&self.input).accepts(prefix);
        // With conditional-lists, so is a name followed by a conditional group whose first
        // arm starts its list (`Balloc #ifdef KR_headers (k) int k; #else (int k) #endif`);
        // definition-bodies then checks for the body.
        let promoted = promoted
            || (matches!(after, After::Call | After::Other)
                && self.corrections.conditional_lists
                && !self.nesting.function
                && inside == 0
                && !self.region.directive
                && can_define
                && may_head(
                    &self.collected.text()[..start],
                    &self.collected.text()[start..],
                )
                && self.bodies.conditional_list(&self.input));
        let after = if promoted { After::Definition } else { after };
        if (self.corrections.dereferenced_calls || self.corrections.split_calls)
            && (matches!(call_prefix, CallPrefix::Boundary)
                || (matches!(call_prefix, CallPrefix::Open | CallPrefix::DeclarationOpen)
                    && self.nesting.calls.closing == Some(self.nesting.parentheses - 1)))
            && matches!(after, After::Declarator)
            && keywords::keyword(&self.collected.text()[start..]).is_none()
            && self.calls_are_recorded()
            && !self.region.macro_body
            && !self.in_directive()
        {
            self.nesting
                .calls
                .begin_declaration(self.nesting.parentheses, self.nesting.depth);
        }
        let after = match after {
            After::Definition if return_type_wrapper => After::Call,
            After::Definition
                if self.corrections.definition_bodies
                    && self.may_define_function()
                    && (self.region.directive
                        || parameter_type
                        || trailing_annotation
                        || is_attribute(&self.collected.text()[start..])
                        || !self.bodies.follow(&self.input, inside)) =>
            {
                // A keyword, or a name inside a definition's parameter list (a parameter's
                // type), is only a name.
                if parameter_type && self.nesting.head_subscripts > 0 {
                    After::Call
                } else if parameter_type || trailing_annotation {
                    After::Other
                } else if keywords::keyword(&self.collected.text()[start..]).is_some() {
                    declared_keyword = !self.region.directive;
                    After::Other
                } else {
                    // In a directive the head is an ordinary call form, as on the
                    // directive's first line. So is a macro among a declaration's
                    // specifiers: an attribute (`__declspec(thread) T name;`), or a head
                    // that begins its declaration in a linkage block
                    // (`PyAPI_DATA(int) name;`), which the reference, one brace deep there,
                    // reads as a call. Its form ends with its list, so the name after it
                    // declares.
                    let specifier = !self.region.directive
                        && inside == 0
                        && (is_attribute(&self.collected.text()[start..])
                            || (!self.nesting.linkage.is_empty() && !self.declaration.begun));
                    numeric_specifier = specifier
                        && !self.nesting.linkage.is_empty()
                        && !self.declaration.begun
                        && self.bodies.numeric_arguments(&self.input);
                    declared_function = !self.region.directive && !specifier;
                    After::Call
                }
            }
            after => after,
        };
        let declaring = self.declaring();
        let prepared = Declarators::prepare_name(
            &mut self.declaration,
            &self.nesting,
            &self.input,
            &self.collected.text()[..start],
            self.corrections,
            declaring,
        );
        let macro_head = self.corrections.macro_function_heads
            && self.region.macro_body
            && matches!(after, After::Definition | After::Call)
            && head_prefix(&self.collected.text()[..start])
            && self.bodies.macro_head(&self.input).accepts(true);
        let macro_constructor = !macro_head
            && self.corrections.macro_function_heads
            && self.region.macro_body
            && matches!(after, After::Definition | After::Call)
            && self.calls_are_recorded()
            && self.call_mark(start) == Mark::Call
            && match self.macro_start {
                Some(MacroStart::Text(from)) => self.bodies.macro_constructor(
                    &self.input,
                    &self.collected.text()[start..],
                    from,
                ),
                _ => false,
            };
        let mut opens_form = false;
        let mark = match after {
            After::Definition | After::Call if macro_head || macro_constructor => Mark::Plain,
            After::Definition if self.may_define_function() => {
                self.nesting.function = true;
                if promoted
                    || (self.corrections.definition_bodies
                        && keywords::keyword(&self.collected.text()[start..]).is_none())
                {
                    self.nesting.head_pending =
                        Some(if prefix && self.bodies.typed_parameters(&self.input) {
                            PendingHead::Written
                        } else {
                            PendingHead::Provisional
                        });
                    self.declaration.type_prefix = false;
                    self.nesting.head_subscripts = 0;
                }
                Mark::Function
            }
            After::Definition | After::Call if self.calls_are_recorded() => self.call_mark(start),
            After::Definition | After::Call => {
                opens_form = true;
                Mark::Plain
            }
            After::Declarator => Mark::Plain,
            After::Other => Mark::Plain,
        };
        let end = self.collected.len();
        let word = &self.collected.text()[start..end];
        let type_word = return_type_wrapper
            || keyword_type(word)
            || (self.nesting.head_pending.is_none()
                && matches!(after, After::Other)
                && keywords::keyword(word).is_none()
                && !matches!(word, b"inline" | b"__inline" | b"__inline__"));
        self.finish_found(
            NameReading::Shaped {
                after,
                ordinary: mark,
                declared_function,
                opens_form,
                prepared,
            },
            start,
            end,
        );
        // A numeric prefix annotation leaves room for a following specifier macro,
        // as in `DEPRECATED(3.12) API_DATA(int) name;` inside a linkage block.
        if !numeric_specifier {
            self.begins_declaration(type_word);
        }
        // The rest of a function declaration that a keyword starts declares nothing.
        if declared_keyword {
            self.declaration
                .declarators
                .keyword_head(self.nesting.parentheses);
        }
    }

    /// Attribute labels never change declaration state. Their expression arguments
    /// can call functions, including between a written head and its body.
    fn attribute_name(&mut self, start: usize) -> bool {
        if !self.corrections.attribute_expressions {
            return false;
        }
        let word = &self.collected.text()[start..];
        let introducer = is_attribute(word);
        if self.nesting.attribute.is_none() && introducer {
            let open = self
                .input
                .run_within(0, DECLARATOR_WINDOW, |byte| is_blank(byte) || byte == b'\n');
            if self.input.at(open) == Some(b'(')
                || declarator_gap(&self.input, 0)
                    .is_some_and(|open| self.input.at(open) == Some(b'('))
            {
                if self.declaration.type_form
                    && self.declaration.typedef == Some(self.nesting.depth)
                    && self.nesting.parentheses > self.nesting.statement_parentheses
                {
                    self.declaration.type_form = false;
                    self.declaration.type_expected = false;
                }
                self.nesting.attribute = Some(Attribute {
                    outside: self.nesting.parentheses,
                    labels: self.nesting.parentheses + if word == b"__declspec" { 1 } else { 2 },
                });
            }
        }
        let Some(attribute) = self.nesting.attribute else {
            return false;
        };
        if let Some(keyword) = keywords::keyword(word) {
            if let Some(code) = keyword.code {
                self.collected.write_code(start, code);
            }
            return true;
        }
        let expression = !introducer && self.nesting.parentheses > attribute.labels;
        let call = expression
            && (self.nesting.function || self.region.macro_body || self.region.rules_section)
            && (self.look.call(&self.input).is_some()
                || self.look.definition(&self.input).is_some());
        let mark = if call {
            self.call_mark(start)
        } else {
            Mark::Plain
        };
        self.collected.anchor = start;
        self.collected.name(mark, start, self.collected.len());
        true
    }

    /// With the definition-bodies Correction, a word just read at its declaration's own
    /// parenthesis level, in code at file scope, means the declaration has begun.
    fn begins_declaration(&mut self, type_word: bool) {
        if self.corrections.definition_bodies
            && self.region.file_scope
            && !self.region.macro_body
            && !self.in_directive()
            && self.nesting.parentheses == self.nesting.statement_parentheses
        {
            self.declaration.begun = true;
            self.declaration.type_prefix |= type_word;
        }
    }

    fn tracks_tails(&self) -> bool {
        self.corrections.bare_directive_words || self.corrections.opaque_member_tails
    }

    fn may_define_function(&self) -> bool {
        let top_level = self.nesting.depth == 0
            && !self.region.macro_body
            && !self.region.rules_section
            && !self.in_directive();
        top_level || (self.nesting.depth == 1 && self.nesting.class_body)
    }

    /// The line starts with a directive, or with the directive-continuations Correction,
    /// is read as part of one: its continuation lines, and the rest of its own line where
    /// the reference reads code (after an `#include` operand, or a `#` in mid-line).
    fn in_directive(&self) -> bool {
        self.collected.is_directive()
            || (self.corrections.directive_continuations && self.region.directive)
    }

    /// Expression keywords and a conditional's preprocessing operator keep their
    /// occurrences without becoming callable names. Operands are read normally.
    fn call_mark(&self, start: usize) -> Mark {
        let word = &self.collected.text()[start..];
        if (self.corrections.c11_expression_keywords && matches!(word, b"_Generic" | b"_Alignof"))
            || (self.corrections.preprocessor_defined
                && self.region.conditional_expression
                && word == b"defined")
        {
            Mark::Plain
        } else {
            Mark::Call
        }
    }

    /// Calls are recorded in a function body, as in a macro body or rules section. While
    /// a head waits for its body (definition-bodies), array bounds are expressions;
    /// calls on directive lines also belong to the function.
    fn calls_are_recorded(&self) -> bool {
        (self.nesting.function
            && (self.nesting.head_pending.is_none()
                || self.nesting.head_subscripts > 0
                || self.region.directive))
            || self.region.macro_body
            || self.region.rules_section
    }

    /// A name read here, at file scope outside a macro, function body or directive, in a
    /// declaration nothing keeps from declaring it (`Declaration::declaring`), declares
    /// something: a global, or a member when an aggregate body is open. Where it stands in
    /// a form decides the rest (`declares_here`, `pointer_declarator`).
    fn declaring(&self) -> bool {
        self.region.file_scope
            && !self.region.macro_body
            && !self.nesting.function
            && !self.in_directive()
            && self.declaration.declaring()
    }

    /// Records the word `start..end` with `mark`, unless it is a keyword; a pending tag
    /// or typedef changes the mark (reading-c.md §5).
    fn found(&mut self, mark: Mark, start: usize, end: usize) {
        self.finish_found(NameReading::Marked(mark), start, end);
    }

    fn finish_found(&mut self, reading: NameReading, start: usize, end: usize) {
        if matches!(reading, NameReading::Marked(Mark::Define)) && self.corrections.keyword_macros {
            self.collected.anchor = start;
            self.collected.name(Mark::Define, start, end);
            return;
        }
        let directive = self.in_directive();
        let finished = Declarators::finish_name(
            &mut self.declaration,
            &self.nesting,
            &self.region,
            &self.input,
            &mut self.look,
            &self.collected.text()[..start],
            &self.collected.text()[start..end],
            reading,
            self.corrections,
            directive,
        );
        let (mark, tag) = match finished {
            FinishedName::Name { mark, tag } => (mark, tag),
            FinishedName::Keyword(keyword) => {
                self.keyword_found(keyword, start);
                return;
            }
        };
        if tag && self.nesting.depth == 0 {
            self.nesting.aggregate = true;
        }
        self.collected.anchor = start;
        // Both readings look at the same window, made once.
        const _: () = assert!(assignments::WINDOW == super::mutations::WINDOW);
        let (subscripts, mutations) = (
            self.corrections.subscript_assignments,
            self.corrections.named_mutations,
        );
        let arrays = self.nesting.calls.arrays > 0;
        let assigned = (subscripts || mutations)
            && self
                .input
                .around_assignment(end - start, assignments::WINDOW)
                .is_some_and(|(before, after)| {
                    (subscripts
                        && (assignments::subscript_store(
                            &before,
                            after,
                            matches!(mark, Mark::Global | Mark::Member),
                        ) || (arrays && assignments::scalar_store(&before, after))))
                        || (mutations && super::mutations::named(&before, after))
                });
        self.collected
            .name_with_assignment(mark, start, end, assigned);
    }

    fn at_typedef_prefix(&self) -> bool {
        self.declaration.at_typedef_prefix(&self.nesting)
    }

    fn block_typedef_prefix(&mut self) {
        if self.at_typedef_prefix() {
            self.declaration.typedef_prefix.words = false;
        }
    }

    fn keyword_found(&mut self, keyword: &Keyword, start: usize) {
        if let Some(code) = keyword.code {
            self.collected.write_code(start, code);
        }
        if keyword.role == Role::Extern
            && self.corrections.linkage_blocks
            && self.bodies.linkage(&self.input)
        {
            self.nesting.linkage_pending = true;
            return;
        }
        let qualifier =
            self.corrections.typedef_qualifiers && matches!(keyword.word, b"const" | b"volatile");
        self.declaration.keyword(
            keyword.role,
            self.nesting.depth,
            self.nesting.parentheses,
            qualifier,
        );
    }

    /// After `struct`, `union` or `enum`, which started at `start` (reading-c.md §5).
    fn after_tag_keyword(&mut self, start: usize) {
        self.declaration.tail.block();
        let tag = self.collected.text()[start];
        let attributed = self
            .corrections
            .attribute_expressions
            .then(|| self.bodies.attributed_tag(&self.input))
            .flatten();
        let candidates = [
            (
                AfterTagKeyword::Definition,
                self.look.tag_definition(&self.input),
            ),
            (AfterTagKeyword::Body, self.look.tagless_body(&self.input)),
            (AfterTagKeyword::Use, self.look.tag_use(&self.input)),
            (AfterTagKeyword::Other, self.input.at(0).map(|_| 1)),
        ];
        let Some((after, _)) = candidates
            .into_iter()
            .filter_map(|(after, reach)| Some((after, reach?)))
            .reduce(|best, candidate| {
                if candidate.1 > best.1 {
                    candidate
                } else {
                    best
                }
            })
        else {
            return;
        };
        let end = self.collected.len();
        self.begins_declaration(true);
        let after = match attributed {
            Some(TagBody::Named) => AfterTagKeyword::Definition,
            Some(TagBody::Anonymous) => AfterTagKeyword::Body,
            None => after,
        };
        if self.nesting.depth == 0 && self.tracks_tails() {
            self.nesting.file_structure = matches!(tag, b's' | b'u')
                && matches!(after, AfterTagKeyword::Definition | AfterTagKeyword::Body);
        }
        match after {
            AfterTagKeyword::Definition => {
                self.declaration.tag = Some(tag);
                self.found(Mark::Plain, start, end);
            }
            AfterTagKeyword::Body => {
                // A body without a tag: the keyword stays as written, never looked up.
                if self.nesting.depth == 0 {
                    self.nesting.aggregate = true;
                }
            }
            AfterTagKeyword::Use | AfterTagKeyword::Other => self.found(Mark::Plain, start, end),
        }
    }

    // Directives.

    /// The word after `#` says what the line is (reading-c.md §6): `endif` and `else`;
    /// `if`, `ifdef`, `ifndef` and `elif` when a blank follows; `include` with a complete
    /// operand; `define` with a blank and a name. Any other word is ordinary text, and a
    /// line end right after `#` stays in the record.
    fn directive(&mut self, byte: u8) {
        self.reading = Reading::Code;
        if byte == b'\n' {
            // The next line joins this record, but the directive ends with its line.
            self.region.directive = false;
            self.region.conditional_expression = false;
            self.line = self.line.saturating_add(1);
            self.take(1);
            return;
        }
        let input = &self.input;
        let length = if is_word_start(byte) {
            input.run(0, is_word)
        } else {
            0
        };
        let word = input.peek(length);
        let blanks = input.run(length, is_blank);
        let rest_of_line = |from: usize| input.run(from, |byte| byte != b'\n');
        // With the unspaced-conditionals Correction, `#if(` and `#elif!` are conditionals.
        let unspaced = self.corrections.unspaced_conditionals
            && input
                .at(length)
                .is_some_and(|byte| !is_word(byte) && !is_blank(byte) && byte != b'\n');
        self.region.conditional_expression = self.collected.text() == b"#"
            && !self.region.macro_body
            && matches!(&word[..], b"if" | b"elif")
            && input.at(length).is_some_and(|byte| !is_word(byte));
        let closed_tail = (self.corrections.conditional_comments
            && matches!(&word[..], b"else" | b"endif"))
        .then(|| closed_conditional_tail(input, length))
        .flatten();
        match &word[..] {
            b"endif" => {
                // Text after `#endif` and a separator belongs to it.
                let tail = match input.at(length) {
                    Some(next) if next != b'\n' => rest_of_line(length),
                    _ => 0,
                };
                if let Some(end) = closed_tail {
                    self.skip_conditional_tail(end);
                } else {
                    self.take(length + tail);
                }
                self.reading = Reading::AfterEndif;
            }
            b"else" => {
                let tail = if blanks > 0 { rest_of_line(length) } else { 0 };
                self.conditionals
                    .alternate(&mut self.nesting, &mut self.declaration, true);
                if let Some(end) = closed_tail {
                    self.skip_conditional_tail(end);
                } else {
                    self.take(length + tail);
                }
            }
            b"if" | b"ifdef" | b"ifndef" if blanks > 0 || (unspaced && &word[..] == b"if") => {
                self.conditionals
                    .open(&mut self.nesting, &mut self.declaration);
                self.take(length + blanks);
            }
            b"elif" if blanks > 0 || unspaced => {
                self.conditionals
                    .alternate(&mut self.nesting, &mut self.declaration, false);
                self.take(length + blanks);
            }
            b"include" => match include_operand(input, length) {
                Some(end) => self.include(end),
                None => self.take(length),
            },
            b"define" if blanks > 0 && input.at(length + blanks).is_some_and(is_word_start) => {
                let name = input.run(length + blanks, is_word);
                self.define(length + blanks + name);
            }
            _ => self.take(length.max(1)),
        }
    }

    /// A proven comment tail has no names or stored records. Preserve its physical
    /// line count so facts after it still point at their original source lines.
    fn skip_conditional_tail(&mut self, length: usize) {
        for _ in 0..length {
            let newline = self.input.at(0) == Some(b'\n');
            self.skip(1);
            if newline {
                self.next_line();
            }
        }
    }

    /// `#include` and its operand: the operand is stored in canonical form.
    fn include(&mut self, length: usize) {
        self.take(length);
        // The operand starts at the line's first quote or angle bracket, wherever that
        // is, as in the reference.
        let text = self.collected.text();
        let (Some(open), Some(&closer)) = (
            text.iter().position(|byte| *byte == b'"' || *byte == b'<'),
            text.last(),
        ) else {
            return;
        };
        let mut operand = paths::canonical(&text[open + 1..text.len() - 1]);
        let end = open + 1 + operand.len();
        operand.push(closer);
        self.collected.rewrite_from(open + 1, &operand);
        self.collected.write_code(0, INCLUDE_CODE);
        self.collected.anchor = open;
        self.collected.name(Mark::Include, open, end);
    }

    /// `#define` and its name; the body is a macro body up to the line end.
    fn define(&mut self, length: usize) {
        let nested = self.region.macro_body;
        self.region.macro_body = true;
        // A `#define` inside another macro's body keeps the state from before the first.
        if self.corrections.macro_bodies && self.before_macro.is_none() {
            self.before_macro = Some((
                std::mem::take(&mut self.nesting),
                std::mem::take(&mut self.declaration),
            ));
        }
        self.collected.write_code(0, DEFINE_CODE);
        let from = self.collected.len();
        self.take(length);
        let end = self.collected.len();
        // The name is taken from after the last space or tab, so other blank bytes after
        // one stay part of it; without either, it starts after the blank run.
        let matched = &self.collected.text()[from..end];
        let start = from
            + matched
                .iter()
                .rposition(|byte| *byte == b' ' || *byte == b'\t')
                .map_or_else(
                    || {
                        6 + matched[6..]
                            .iter()
                            .take_while(|byte| is_blank(**byte))
                            .count()
                    },
                    |last| last + 1,
                );
        self.found(Mark::Define, start, end);
        if self.input.at(0) == Some(b'(') {
            self.nesting.calls.closing = Some(self.nesting.parentheses);
            if !nested {
                self.macro_start = Some(MacroStart::Parameters(self.nesting.parentheses));
            }
        } else if !nested {
            self.macro_start = self.input.source_position(0).map(MacroStart::Header);
        }
    }

    /// After `#endif`: a following `#if` of the same shape continues the group unless it
    /// had an `#else`; otherwise the group closes (reading-c.md §6).
    fn after_endif(&mut self) {
        self.conditionals.close(
            &mut self.nesting,
            &mut self.declaration,
            &self.input,
            &mut self.look,
        );
        self.reading = Reading::Code;
        if self.input.at(0) == Some(b'\n') {
            self.skip(1);
            self.next_line();
        }
    }

    // Literals and comments.

    fn literal(&mut self, byte: u8, closer: u8) {
        match byte {
            _ if byte == closer => {
                self.take(1);
                self.reading = Reading::Code;
            }
            b'\n' => {
                self.reading = Reading::Code;
                // An escaped pair may already have consumed the physical backslash
                // before this newline. That splice still continues the macro.
                let spliced = self.collected.text().ends_with(b"\\")
                    || (self.corrections.crlf_continuations
                        && self.collected.text().ends_with(b"\\\r"));
                if self.corrections.macro_bodies && self.region.macro_body && !spliced {
                    // An unfinished quoted token cannot continue a replacement list
                    // across an unspliced line end.
                    self.line_end();
                } else {
                    self.skip(1);
                    self.next_line();
                }
            }
            b'\\' if self.input.at(1) == Some(b'\n') => {
                // Continued inside a literal: the raw line end stays in the record.
                self.line = self.line.saturating_add(1);
                self.take(2);
            }
            b'\\'
                if self.corrections.crlf_continuations
                    && self.input.at(1) == Some(b'\r')
                    && self.input.at(2) == Some(b'\n') =>
            {
                // As before LF: the CR is dropped, so the record is the LF original's.
                self.line = self.line.saturating_add(1);
                self.take(1);
                self.skip(1);
                self.take(1);
            }
            b'\\' if self.input.at(1).is_some() => self.take(2),
            _ => self.take(1),
        }
    }

    fn comment(&mut self, byte: u8) {
        match byte {
            b'*' => {
                let stars = self.input.run(0, |byte| byte == b'*');
                if self.input.at(stars) == Some(b'/') {
                    // The comment reads as one blank.
                    self.skip(stars + 1);
                    self.reading = Reading::Code;
                    self.input.push_back(b" ");
                } else {
                    self.skip(stars);
                }
            }
            b'\n' => {
                self.skip(1);
                if self.region.macro_body {
                    self.line = self.line.saturating_add(1);
                } else {
                    self.next_line();
                }
            }
            _ => self.skip(1),
        }
    }
}

/// Only the closing name group and opening argument list are needed, within the
/// existing window. Arguments themselves are read normally, without resolving targets.
fn dereferenced_suffix(input: &Input) -> bool {
    let Some(close) = declarator_gap(input, 0) else {
        return false;
    };
    input.at(close) == Some(b')')
        && declarator_gap(input, close + 1).is_some_and(|open| input.at(open) == Some(b'('))
}

/// Keep the declarator gap's conservative comment/splice boundaries. A written type
/// or qualifier cannot start a call argument: an annotation before a cast stays plain.
/// The gap, opening parenthesis and initial argument check share the same window.
fn split_call_suffix(input: &Input) -> bool {
    if input.source_position(0).is_none() {
        return false;
    }
    let Some(open) = declarator_gap(input, 0) else {
        return false;
    };
    if input.at(open) != Some(b'(') || !input.peek_at(0, open).contains(&b'\n') {
        return false;
    }
    let Some(first) = declarator_gap(input, open + 1) else {
        return false;
    };
    let length = input.run_within(first, DECLARATOR_WINDOW, is_word);
    let word = input.peek_at(first, length);
    let type_start = keywords::keyword_type(&word)
        || matches!(
            &word[..],
            b"_Bool"
                | b"_Complex"
                | b"_Imaginary"
                | b"_Atomic"
                | b"const"
                | b"volatile"
                | b"restrict"
                | b"struct"
                | b"union"
                | b"enum"
        );
    first + length < DECLARATOR_WINDOW && !type_start
}

/// A simple atomic type operand: words and pointer stars, with no nested declarator,
/// expression, directive or aggregate body. No type lookup or macro expansion is needed.
fn simple_atomic_type(input: &Input, from: usize) -> bool {
    let Some(open) = declarator_gap(input, from) else {
        return false;
    };
    if input.at(open) != Some(b'(') {
        return false;
    }
    let (mut at, mut word) = (open + 1, false);
    while let Some(next) = declarator_gap(input, at) {
        at = next;
        match input.at(at) {
            Some(byte) if is_word_start(byte) => {
                let length = input.run_within(at, DECLARATOR_WINDOW, is_word);
                if at + length >= DECLARATOR_WINDOW {
                    return false;
                }
                let spelling = input.peek_at(at, length);
                if keywords::keyword(&spelling).is_some()
                    && !keyword_type(&spelling)
                    && !matches!(
                        &spelling[..],
                        b"struct" | b"union" | b"enum" | b"const" | b"volatile"
                    )
                {
                    return false;
                }
                word = true;
                at += length;
            }
            Some(b'*') if word => at += 1,
            Some(b')') => return word,
            _ => return false,
        }
    }
    false
}
