//! Bounded declarator readings and the lifetime of their forms and parameter lists.
//! Source consumption, ordinary head decisions and record emission stay in Scan.

use super::{After, Declaration, Nesting, Region, TypedefPrefix, is_attribute};
use crate::scanner::{
    Corrections,
    input::Input,
    keywords::{self, Keyword, Role, keyword_type},
    shapes::{LookAhead, is_blank, is_word, is_word_start},
};
use crate::unit::Mark;

/// Form and list state travels with Declaration in macro snapshots. Conditional
/// arms continue to restore only their existing shared declaration evidence.
#[derive(Clone, Default)]
pub(super) struct Declarators {
    /// The parenthesis depth of a call-like form read outside any function, until its
    /// parenthesis closes or a keyword follows at that depth.
    form: Option<i32>,
    /// The form is a function declaration's (the definition-bodies Correction): it lasts
    /// past its parenthesis, to a `;` or `,` at its depth or outside it, or to the `)`
    /// that closes a parenthesis around it.
    declared_function: bool,
    /// The form is a parenthesised declarator's parameter list (the definition-bodies
    /// Correction), opened at this brace depth: it also ends at a `;` or `}` of that
    /// block, whatever the parenthesis depth, so a parenthesis left open by `#if` arms
    /// cannot carry it on, and at a `{` outside the list.
    declarator_list: Option<i32>,
    /// The source position of a proven pointer declarator's parameter list on a later
    /// line or after a block comment. Only whitespace and closed comments separate it
    /// from the declarator's closing parenthesis.
    split_parameter_start: Option<usize>,
    /// The parenthesis depth before a recognised function typedef's parameter list.
    /// Names inside that list are not typedefs (typedef-parameters).
    typedef_parameters: Option<i32>,
    /// The source position of a recognised parenthesised typedef's following list.
    typedef_parameter_start: Option<usize>,
}

/// Preparation retains only the supported reading, never a source offset for Scan.
pub(super) struct PreparedName {
    split: bool,
}

pub(super) enum NameReading {
    Marked(Mark),
    Shaped {
        after: After,
        ordinary: Mark,
        declared_function: bool,
        opens_form: bool,
        prepared: PreparedName,
    },
}

pub(super) enum FinishedName {
    Keyword(&'static Keyword),
    Name { mark: Mark, tag: bool },
}

impl Declarators {
    /// Install deferred split-list evidence before ordinary macro/head marking.
    pub(super) fn prepare_name(
        declaration: &mut Declaration,
        nesting: &Nesting,
        input: &Input,
        before: &[u8],
        corrections: Corrections,
        declaring: bool,
    ) -> PreparedName {
        let split = declaration.declarators.split_pointer_parameters(
            declaration,
            nesting,
            input,
            before,
            corrections,
            declaring,
        );
        if let Some(position) = split {
            declaration.declarators.split_parameter_start = Some(position);
        }
        PreparedName {
            split: split.is_some(),
        }
    }

    /// Apply object and typedef refinements in their original order. Keywords
    /// return to Scan for rendering and keyword lifecycle handling.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_name(
        declaration: &mut Declaration,
        nesting: &Nesting,
        region: &Region,
        input: &Input,
        look: &mut LookAhead,
        before: &[u8],
        word: &[u8],
        reading: NameReading,
        corrections: Corrections,
        directive: bool,
    ) -> FinishedName {
        let declaring = region.file_scope
            && !region.macro_body
            && !nesting.function
            && !directive
            && declaration.declaring();
        let inside = (nesting.parentheses - nesting.statement_parentheses).max(0);
        let mark = match reading {
            NameReading::Marked(mark) => mark,
            NameReading::Shaped {
                after,
                ordinary,
                declared_function,
                opens_form,
                prepared,
            } => match after {
                After::Definition | After::Call if opens_form => {
                    if declaration.declarators.form.is_none() {
                        declaration.declarators.form = Some(nesting.parentheses);
                        declaration.declarators.declared_function = declared_function;
                    }
                    ordinary
                }
                After::Other
                    if corrections.pointer_declarators
                        && declaring
                        && inside == 0
                        && tagged_pointer_return(input) =>
                {
                    Mark::Plain
                }
                After::Other
                    if (declaring && declaration.declarators.form.is_none())
                        || declaration.declarators.pointer_object(
                            nesting,
                            input,
                            before,
                            corrections,
                            declaring,
                        )
                        || (corrections.pointer_declarators && prepared.split) =>
                {
                    if nesting.aggregate {
                        Mark::Member
                    } else {
                        Mark::Global
                    }
                }
                After::Other => Mark::Plain,
                _ => ordinary,
            },
        };
        if let Some(keyword) = keywords::keyword(word) {
            return FinishedName::Keyword(keyword);
        }
        // With the parenthesised-typedefs Correction, the keyword whose call form ended
        // before this name was the typedef's type when this is its group's name.
        if std::mem::take(&mut declaration.type_form)
            && corrections.parenthesised_typedefs
            && names_group(before, input)
        {
            declaration.type_expected = false;
        }
        // With the typedef-qualifiers Correction, the name a qualifier left the type to is
        // the aliased type's, a plain name, unless it is the declarator itself. A tag being
        // defined, a macro's name, or a name deeper than the typedef, leaves the decision
        // to the next name.
        let qualified = mark != Mark::Define
            && declaration.tag.is_none()
            && declaration.typedef == Some(nesting.depth)
            && std::mem::take(&mut declaration.qualified)
            && declaration.type_expected;
        if qualified && declarator_ends(input) {
            declaration.type_expected = false;
        }
        let bounded_before = &before[before.len().saturating_sub(DECLARATOR_WINDOW)..];
        let tag_use = corrections.typedef_parameters
            && declaration.typedef == Some(nesting.depth)
            && declaration.type_expected
            && declaration.tag.is_none()
            && matches!(
                bounded_before
                    .trim_ascii_end()
                    .rsplit(|byte| !is_word(*byte))
                    .next(),
                Some(b"struct" | b"union" | b"enum")
            )
            && tagged_pointer_return(input);
        let (mark, tag) = declaration.name(word, mark, nesting.depth);
        let mut mark = if tag_use
            || (qualified && mark != Mark::Typedef)
            || opaque_tail(
                declaration,
                nesting,
                region,
                input,
                corrections,
                directive,
                mark,
            ) {
            Mark::Plain
        } else {
            mark
        };
        if corrections.typedef_decorations
            && mark == Mark::Typedef
            && declaration.at_typedef_prefix(nesting)
            && declaration.typedef_prefix.words
            && !declaration.typedef_prefix.pointer
            && !region.macro_body
            && !directive
            && later_typedef_declarator(input)
        {
            mark = Mark::Plain;
        }
        if corrections.typedef_parameters
            && mark == Mark::Typedef
            && !region.macro_body
            && !directive
        {
            if declaration
                .declarators
                .typedef_parameters
                .is_some_and(|depth| nesting.parentheses > depth)
            {
                mark = Mark::Plain;
            } else if let Some((part, at)) = opaque_typedef_group(before, input) {
                mark = part;
                declaration.declarators.typedef_parameter_start = input.source_position(at);
            } else if typed_parameters(input, 0) && look.call(input).is_some() {
                declaration.declarators.typedef_parameters = Some(nesting.parentheses);
            } else if let Some(at) = keyword_declarator_parameters(before, input)
                .or_else(|| parenthesised_typedef_parameters(bounded_before, input))
            {
                declaration.declarators.typedef_parameter_start = input.source_position(at);
            } else if bounded_before
                .iter()
                .rposition(|byte| *byte == b'(')
                .is_some_and(|open| {
                    bounded_before[open + 1..]
                        .iter()
                        .all(|byte| is_blank(*byte) || is_word(*byte))
                })
                && typedef_pointer_decoration(input, 0)
            {
                mark = Mark::Plain;
            }
        }

        FinishedName::Name { mark, tag }
    }

    fn pointer_object(
        &self,
        nesting: &Nesting,
        input: &Input,
        before: &[u8],
        corrections: Corrections,
        declaring: bool,
    ) -> bool {
        corrections.pointer_declarators
            && declaring
            && self.declarator_list.is_none()
            && self.form.is_some_and(|depth| {
                depth == nesting.statement_parentheses && depth + 1 == nesting.parentheses
            })
            && (opens_pointer_group(before, false)
                || ((nesting.depth == 0 || nesting.aggregate)
                    && opens_pointer_group(before, true)
                    && parameter_suffix(input)))
            && closes_before_suffix(input)
    }

    fn split_pointer_parameters(
        &self,
        declaration: &Declaration,
        nesting: &Nesting,
        input: &Input,
        before: &[u8],
        corrections: Corrections,
        declaring: bool,
    ) -> Option<usize> {
        if !corrections.definition_bodies
            || !declaring
            || declaration.typedef == Some(nesting.depth)
            || self.declarator_list.is_some()
            || nesting.parentheses != 1
            || nesting.parentheses != nesting.statement_parentheses + 1
            || self
                .form
                .is_some_and(|depth| depth != nesting.statement_parentheses)
            || !(nesting.depth == 0 || nesting.aggregate)
            || !opens_pointer_group(before, true)
        {
            return None;
        }
        let close = group_close(input)?;
        let open = declarator_gap(input, close)?;
        if !(close..open).any(|at| matches!(input.at(at), Some(b'\n' | b'/')))
            || !proved_declarator_parameters(before, input, open)
        {
            return None;
        }
        input.source_position(open)
    }

    /// A keyword at brace depth `depth` and parenthesis depth `parentheses`. A
    /// `qualifier` (with the typedef-qualifiers Correction) is never a typedef's type.
    pub(super) fn keyword(
        declaration: &mut Declaration,
        role: Role,
        depth: i32,
        parentheses: i32,
        qualifier: bool,
    ) {
        if qualifier && declaration.type_expected && declaration.typedef == Some(depth) {
            declaration.qualified = true;
        }
        match role {
            Role::Typedef => {
                declaration.typedef = Some(depth);
                declaration.declarators.typedef_parameters = None;
                declaration.declarators.typedef_parameter_start = None;
                declaration.type_expected = true;
                declaration.type_form = false;
                declaration.qualified = false;
                declaration.typedef_prefix = TypedefPrefix {
                    words: true,
                    pointer: false,
                };
            }
            Role::Extern => declaration.extern_depth = Some(depth),
            Role::Tag => {}
            // A type or statement word at the form's own depth means the form was not a
            // declaration's head after all. Ending a function declaration's form (the
            // definition-bodies Correction), it is also the type word a pending `typedef`
            // waits for, as it is where no such form was read (`DECLARE(x)` then
            // `typedef void (*F)(void);`). Ending its own call form (`typedef void
            // (*f)();`), it leaves the decision to the next name.
            Role::Other if declaration.declarators.form == Some(parentheses) => {
                if declaration.declarators.declared_function {
                    if !qualifier {
                        declaration.type_expected = false;
                    }
                } else if declaration.type_expected
                    && declaration.typedef == Some(depth)
                    && !qualifier
                {
                    declaration.type_form = true;
                }
                declaration.declarators.form = None;
                declaration.declarators.declared_function = false;
            }
            Role::Other if qualifier => {}
            Role::Other => declaration.type_expected = false,
        }
    }

    pub(super) fn reset_typedef(&mut self) {
        self.typedef_parameters = None;
        self.typedef_parameter_start = None;
    }

    pub(super) fn keyword_head(&mut self, parentheses: i32) {
        if self.form.is_none() {
            self.form = Some(parentheses);
            self.declared_function = true;
        }
    }

    /// Before the brace depth changes, terminate a list outside its parentheses.
    pub(super) fn before_body(&mut self, depth: i32, parentheses: i32) {
        if self.form.is_some_and(|depth| parentheses <= depth) {
            self.end_declarator_list(depth);
        }
    }

    /// After opening the body, end a declared head without clearing list evidence.
    pub(super) fn begin_body(&mut self) {
        if self.declared_function {
            self.form = None;
            self.declared_function = false;
        }
    }

    pub(super) fn open_list(
        &mut self,
        nesting: &Nesting,
        region: &Region,
        input: &Input,
        before: &[u8],
        corrections: Corrections,
    ) {
        if self.split_parameter_start.is_some()
            && self.split_parameter_start == input.source_position(0)
        {
            self.split_parameter_start = None;
            self.form = Some(nesting.parentheses);
            self.declared_function = true;
            self.declarator_list = Some(nesting.depth);
        }
        if self.typedef_parameter_start.is_some()
            && self.typedef_parameter_start == input.source_position(0)
        {
            self.typedef_parameters = Some(nesting.parentheses);
            self.typedef_parameter_start = None;
        }
        // With the definition-bodies Correction the parameter list of a
        // parenthesised declarator (`(*name)(int x)`, `(name)(int x)`) declares
        // nothing: a `(` right after `)`, unless `*` or `^` follows it, which
        // starts a declarator group itself (`MACRO(int) (*hook)(void)`).
        if corrections.definition_bodies
            && self.form.is_none()
            && region.file_scope
            && !region.macro_body
            && !nesting.function
            && before.iter().rev().find(|byte| !is_blank(**byte)) == Some(&b')')
            && !matches!(input.at(1 + input.run(1, is_blank)), Some(b'*' | b'^'))
        {
            self.form = Some(nesting.parentheses);
            self.declared_function = proved_declarator_parameters(before, input, 0);
            self.declarator_list = Some(nesting.depth);
        }
    }

    /// Called after the general parenthesis counter and attribute state close.
    pub(super) fn close_list(&mut self, inside: i32, outside: i32) {
        if self
            .typedef_parameters
            .is_some_and(|depth| outside <= depth)
        {
            self.typedef_parameters = None;
        }
        let ends = if self.declared_function {
            // A parenthesis around the declaration closes.
            self.form.is_some_and(|depth| depth >= inside)
        } else {
            self.form == Some(outside)
        };
        if ends {
            self.form = None;
            self.declared_function = false;
            self.declarator_list = None;
        }
    }

    /// A `;` or `}` of the block at brace depth `depth` ends a declarator list's form
    /// opened there.
    pub(super) fn end_declarator_list(&mut self, depth: i32) {
        if self.declarator_list == Some(depth) {
            self.form = None;
            self.declared_function = false;
            self.declarator_list = None;
        }
    }

    /// A `;` or `,` at parenthesis depth `parentheses` ends a function declaration's form
    /// at that depth or inside it.
    pub(super) fn end_declared_function(&mut self, parentheses: i32) {
        if self.declared_function && self.form.is_some_and(|depth| parentheses <= depth) {
            self.form = None;
            self.declared_function = false;
            self.declarator_list = None;
        }
    }
}

fn opaque_tail(
    declaration: &Declaration,
    nesting: &Nesting,
    region: &Region,
    input: &Input,
    corrections: Corrections,
    directive: bool,
    mark: Mark,
) -> bool {
    if region.macro_body || directive || nesting.parentheses != 0 {
        return false;
    }
    let delimiter = match mark {
        Mark::Global
            if corrections.bare_directive_words
                && nesting.depth == 0
                && declaration.tail.before_directive() =>
        {
            b'#'
        }
        Mark::Member
            if corrections.opaque_member_tails
                && nesting.depth == 1
                && nesting.file_structure
                && declaration.tail.before_member_end() =>
        {
            b'}'
        }
        _ => return false,
    };
    let at = input.run_within(0, DECLARATOR_WINDOW, |byte| is_blank(byte) || byte == b'\n');
    at < DECLARATOR_WINDOW
        && input.at(at) == Some(delimiter)
        && (delimiter != b'#' || (0..at).any(|offset| input.at(offset) == Some(b'\n')))
}

/// Each forward and backward declarator proof keeps its independent byte bound.
pub(super) const DECLARATOR_WINDOW: usize = 256;

/// Qualifiers a declarator group may hold among its stars.
const QUALIFIERS: [&[u8]; 9] = [
    b"const",
    b"volatile",
    b"restrict",
    b"__const",
    b"__const__",
    b"__volatile",
    b"__volatile__",
    b"__restrict",
    b"__restrict__",
];

/// A direct typedef's list starts with a parameter type, optionally qualified. A
/// parenthesised alias or decorated pointer group cannot start with a type keyword.
/// Unknown words and comments stay unchanged, including a word cut by the window.
fn typed_parameters(input: &Input, from: usize) -> bool {
    let open = from + input.run_within(from, DECLARATOR_WINDOW, is_blank);
    if input.at(open) != Some(b'(') {
        return false;
    }
    let mut at = open + 1;
    loop {
        at += input.run_within(at, DECLARATOR_WINDOW, |byte| {
            is_blank(byte) || byte == b'\n'
        });
        let length = input.run_within(at, DECLARATOR_WINDOW, is_word);
        if length == 0 || input.at(at + length).is_some_and(is_word) {
            return false;
        }
        let word = input.peek_at(at, length);
        if keyword_type(&word) || matches!(&word[..], b"struct" | b"union" | b"enum") {
            return true;
        }
        if !QUALIFIERS.contains(&&word[..]) {
            return false;
        }
        at += length;
    }
}

/// A decorated group's suffix must be a parameter list, not a parenthesised name
/// after a type expression (`TYPEOF(left *right) (alias)`). An empty list, a keyword
/// type or a comma outside nested groups distinguishes it. All reads stay within the
/// declarator window; opaque one-parameter lists and comments retain the old reading.
fn parameter_suffix(input: &Input) -> bool {
    let Some(end) = group_close(input) else {
        return false;
    };
    let open = end + input.run_within(end, DECLARATOR_WINDOW, is_blank);
    parameter_list(input, open)
}

/// Whether the suffix at `open` shows a parameter list within the declarator window.
fn parameter_list(input: &Input, open: usize) -> bool {
    if open >= DECLARATOR_WINDOW || input.at(open) != Some(b'(') {
        return false;
    }
    if typed_parameters(input, open) {
        return true;
    }
    let start = open + 1;
    let start = start
        + input.run_within(start, DECLARATOR_WINDOW, |byte| {
            is_blank(byte) || byte == b'\n'
        });
    if start < DECLARATOR_WINDOW && input.at(start) == Some(b')') {
        return true;
    }
    let (mut parentheses, mut brackets) = (1, 0);
    for at in start..DECLARATOR_WINDOW {
        match input.at(at) {
            Some(b'(') => parentheses += 1,
            Some(b')') => {
                parentheses -= 1;
                if parentheses == 0 {
                    return false;
                }
            }
            Some(b'[') => brackets += 1,
            Some(b']') if brackets > 0 => brackets -= 1,
            Some(b',') if parentheses == 1 && brackets == 0 => return true,
            None | Some(b'/' | b'\'' | b'"' | b'{' | b'}' | b';' | b'#') => return false,
            Some(_) => {}
        }
    }
    false
}

/// A complete suffix without an inline aggregate body. Those bodies can declare real
/// members, which the existing declarator-list state would otherwise suppress. Complex
/// lists and lists extending past the window keep their previous reading.
fn closed_parameter_list(input: &Input, open: usize) -> bool {
    delimited_end(input, open, b'(', b')').is_some()
}

fn delimited_end(input: &Input, open: usize, left: u8, right: u8) -> Option<usize> {
    let mut depth = 0;
    for at in open..DECLARATOR_WINDOW {
        match input.at(at) {
            Some(byte) if byte == left => depth += 1,
            Some(byte) if byte == right => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            None | Some(b'/' | b'\'' | b'"' | b'{' | b'}' | b';' | b'#') => return None,
            Some(_) => {}
        }
    }
    None
}

/// The word-prefix preference in a typedef: earlier words are uses
/// when a later supported declarator ends the list. Opaque suffix macros can
/// reverse these roles; this selects written syntax, without expanding them.
fn later_typedef_declarator(input: &Input) -> bool {
    let Some(mut at) = declarator_gap(input, 0) else {
        return false;
    };
    let (mut name, mut pointer, mut pointer_name) = (false, false, false);
    while at < DECLARATOR_WINDOW {
        match input.at(at) {
            Some(byte) if is_word_start(byte) => {
                let length = input.run_within(at, DECLARATOR_WINDOW, is_word);
                if input.at(at + length).is_some_and(is_word) {
                    return false;
                }
                let word = input.peek_at(at, length);
                if keywords::keyword(&word).is_some() {
                    if !pointer || !QUALIFIERS.contains(&&word[..]) {
                        return false;
                    }
                } else {
                    if is_attribute(&word) || (pointer && pointer_name) {
                        return false;
                    }
                    name = true;
                    pointer_name |= pointer;
                }
                at += length;
            }
            Some(b'*') if !pointer_name => {
                pointer = true;
                name = false;
                at += 1;
            }
            Some(b'(') => {
                let open = if name {
                    at
                } else {
                    let Some(open) = pointer_group_parameters(input, at + 1) else {
                        return false;
                    };
                    open
                };
                if !parameter_list(input, open) {
                    return false;
                }
                let Some(end) = delimited_end(input, open, b'(', b')') else {
                    return false;
                };
                let Some(end) = declarator_gap(input, end) else {
                    return false;
                };
                return matches!(input.at(end), Some(b';' | b','));
            }
            Some(b'[') if name => {
                let Some(end) = delimited_end(input, at, b'[', b']') else {
                    return false;
                };
                at = end;
            }
            Some(b';' | b',') => return name,
            _ => return false,
        }
        let Some(next) = declarator_gap(input, at) else {
            return false;
        };
        at = next;
    }
    false
}

/// A parenthesised typedef's parameter list is unambiguous when its group immediately
/// follows a keyword type. A word such as `TYPEOF` before `(*p)` might instead be a
/// type macro whose following `(alias)` declares the typedef, so it stays unchanged.
/// Both sides of the name use the existing declarator window and stay on its line.
fn keyword_declarator_parameters(before: &[u8], input: &Input) -> Option<usize> {
    let truncated = before.len() > DECLARATOR_WINDOW;
    let before = &before[before.len().saturating_sub(DECLARATOR_WINDOW)..];
    if !before.trim_ascii_end().ends_with(b"(") && !opens_pointer_group(before, false) {
        return None;
    }
    let group = before.iter().rposition(|byte| *byte == b'(')?;
    let end = before[..group].iter().rposition(|byte| !is_blank(*byte))? + 1;
    let start = before[..end]
        .iter()
        .rposition(|byte| !is_word(*byte))
        .map_or(0, |at| at + 1);
    if truncated && start == 0 {
        return None;
    }
    if !keyword_type(&before[start..end]) {
        return None;
    }
    let end = group_close(input)?;
    let at = end + input.run_within(end, DECLARATOR_WINDOW, is_blank);
    (input.at(at) == Some(b'(')).then_some(at)
}

/// A plain or decorated parenthesised typedef name can own a proved parameter list
/// regardless of its return type. Type expressions followed by an alias, such as
/// `TYPEOF(*ptr) (alias)`, fail the parameter-list proof and keep their reading.
fn parenthesised_typedef_parameters(before: &[u8], input: &Input) -> Option<usize> {
    if !before.trim_ascii_end().ends_with(b"(") && !opens_pointer_group(before, true) {
        return None;
    }
    let close = group_close(input)?;
    let open = close + input.run_within(close, DECLARATOR_WINDOW, is_blank);
    (parameter_list(input, open) && closed_parameter_list(input, open)).then_some(open)
}

/// A written type followed by `(decoration alias)(parameters)` still names its
/// alias when the decoration hides a pointer operator. Only the written name and
/// complete parameter list matter; no macro spelling or expansion is inferred.
fn opaque_typedef_group(before: &[u8], input: &Input) -> Option<(Mark, usize)> {
    if !written_type_before_group(before) {
        return None;
    }
    let before = &before[before.len().saturating_sub(DECLARATOR_WINDOW)..];
    let group = before.iter().rposition(|byte| *byte == b'(')?;
    let preceding = before[group + 1..].trim_ascii();
    let mut at = input.run_within(0, DECLARATOR_WINDOW, is_blank);
    let part = if preceding.is_empty() {
        // This word decorates the one written alias before the closing group.
        if at == 0 || !input.at(at).is_some_and(is_word_start) {
            return None;
        }
        let length = input.run_within(at, DECLARATOR_WINDOW, is_word);
        if keywords::keyword(&input.peek_at(at, length)).is_some() {
            return None;
        }
        at += length;
        at += input.run_within(at, DECLARATOR_WINDOW, is_blank);
        Mark::Plain
    } else if preceding.iter().all(|byte| is_word(*byte))
        && is_word_start(preceding[0])
        && keywords::keyword(preceding).is_none()
    {
        Mark::Typedef
    } else {
        return None;
    };
    if at >= DECLARATOR_WINDOW || input.at(at) != Some(b')') {
        return None;
    }
    let open = at + 1 + input.run_within(at + 1, DECLARATOR_WINDOW, is_blank);
    proved_declarator_parameters(before, input, open).then_some((part, open))
}

/// A builtin type or explicit return-type pointer before a group excludes a type
/// macro's argument (`TYPEOF(*ptr)`). Without that evidence, `(API alias)` and
/// `(API *alias)` can declare aliases rather than parameters.
fn written_type_before_group(before: &[u8]) -> bool {
    let truncated = before.len() > DECLARATOR_WINDOW;
    let before = &before[before.len().saturating_sub(DECLARATOR_WINDOW)..];
    let Some(group) = before.iter().rposition(|byte| *byte == b'(') else {
        return false;
    };
    let prefix = before[..group].trim_ascii_end();
    if prefix.ends_with(b"*") {
        return true;
    }
    let start = prefix
        .iter()
        .rposition(|byte| !is_word(*byte))
        .map_or(0, |at| at + 1);
    !(truncated && start == 0) && keyword_type(&prefix[start..])
}

/// In addition to the usual list evidence, a leading type word followed by `*`
/// or a name proves a parameter (`node *n`, `size_t bytes`) when a written type
/// establishes the preceding group as a declarator. Otherwise that list could be
/// an alias after a type expression. The whole list must fit the window.
fn proved_declarator_parameters(before: &[u8], input: &Input, open: usize) -> bool {
    if open >= DECLARATOR_WINDOW || input.at(open) != Some(b'(') {
        return false;
    }
    let start = open + 1;
    let start = start + input.run_within(start, DECLARATOR_WINDOW, is_blank);
    let length = input.run_within(start, DECLARATOR_WINDOW, is_word);
    let after = start + length;
    let after = after + input.run_within(after, DECLARATOR_WINDOW, is_blank);
    let named_type = input.at(start).is_some_and(is_word_start)
        && length > 0
        && (input.at(after) == Some(b'*')
            || (after > start + length && input.at(after).is_some_and(is_word_start)));
    (parameter_list(input, open) || (named_type && written_type_before_group(before)))
        && closed_parameter_list(input, open)
}

/// A word before the explicit pointer operator decorates the group; it does not
/// declare another typedef. A complete parameter suffix distinguishes the group
/// from an arbitrary macro argument. Comments and nested declarators stay unchanged.
fn typedef_pointer_decoration(input: &Input, from: usize) -> bool {
    pointer_group_parameters(input, from)
        .is_some_and(|open| parameter_list(input, open) && closed_parameter_list(input, open))
}

fn pointer_group_parameters(input: &Input, from: usize) -> Option<usize> {
    let (mut at, mut pointer, mut name) = (from, false, false);
    while at < DECLARATOR_WINDOW {
        match input.at(at) {
            Some(byte) if is_blank(byte) => at += 1,
            Some(b'*' | b'^') if !name => {
                pointer = true;
                at += 1;
            }
            Some(byte) if is_word_start(byte) && !name => {
                let length = input.run_within(at, DECLARATOR_WINDOW, is_word);
                let word = input.peek_at(at, length);
                name = pointer && !QUALIFIERS.contains(&&word[..]);
                at += length;
            }
            Some(b')') if pointer && name => {
                let open = at + 1 + input.run_within(at + 1, DECLARATOR_WINDOW, is_blank);
                return Some(open);
            }
            _ => return None,
        }
    }
    None
}

/// A type word before a pointer-return declarator is a use, not a definition.
/// Prove the following declarator as well: a tag followed by directives may be
/// the name of an aggregate whose body is in another conditional arm.
fn tagged_pointer_return(input: &Input) -> bool {
    let (mut at, mut pointer) = (0, false);
    while at < DECLARATOR_WINDOW {
        match input.at(at) {
            Some(byte) if is_blank(byte) => at += 1,
            Some(b'*') => {
                pointer = true;
                at += 1;
            }
            Some(byte) if pointer && is_word_start(byte) => {
                let length = input.run_within(at, DECLARATOR_WINDOW, is_word);
                if !QUALIFIERS.contains(&&input.peek_at(at, length)[..]) {
                    return false;
                }
                at += length;
            }
            Some(b'(') if pointer => {
                return pointer_group_parameters(input, at + 1)
                    .is_some_and(|open| closed_parameter_list(input, open));
            }
            _ => return false,
        }
    }
    false
}

/// Whitespace and closed block comments before a declarator list or attribute group.
/// Do not read past the declarator window or through line splices inside comments.
pub(super) fn declarator_gap(input: &Input, from: usize) -> Option<usize> {
    let mut at = from;
    while at < DECLARATOR_WINDOW {
        at += input.run_within(at, DECLARATOR_WINDOW, |byte| {
            is_blank(byte) || byte == b'\n'
        });
        if at >= DECLARATOR_WINDOW {
            return None;
        }
        if input.at(at) != Some(b'/') {
            return Some(at);
        }
        if at + 1 >= DECLARATOR_WINDOW {
            return None;
        }
        if input.at(at + 1) != Some(b'*') {
            return Some(at);
        }
        at += 2;
        loop {
            if at + 1 >= DECLARATOR_WINDOW {
                return None;
            }
            match (input.at(at), input.at(at + 1)) {
                (Some(b'*'), Some(b'/')) => {
                    at += 2;
                    break;
                }
                (None, _) | (Some(b'\\'), Some(b'\n' | b'\r')) => return None,
                _ => at += 1,
            }
        }
    }
    None
}

/// Whether the text before a name ends with `(`, one or more `*` or `^`, and qualifiers
/// or blanks between them: a pointer declarator group opened on this line, within
/// `DECLARATOR_WINDOW` bytes of the name. When allowed, decoration words may precede
/// the first star; they cannot replace it or appear among the stars.
fn opens_pointer_group(before: &[u8], allow_decorations: bool) -> bool {
    // A word cut at the window's edge can only end the look: without its `(` there is no
    // group.
    let before = &before[before.len().saturating_sub(DECLARATOR_WINDOW)..];
    let mut end = before.len();
    let mut stars = 0;
    let mut decoration = false;
    loop {
        let Some(&byte) = end.checked_sub(1).and_then(|last| before.get(last)) else {
            return false;
        };
        if is_blank(byte) {
            end -= 1;
        } else if byte == b'*' || byte == b'^' {
            if decoration {
                return false;
            }
            stars += 1;
            end -= 1;
        } else if byte == b'(' {
            return stars > 0;
        } else if is_word(byte) {
            let word = before[..end]
                .iter()
                .rposition(|byte| !is_word(*byte))
                .map_or(0, |last| last + 1);
            if !QUALIFIERS.contains(&&before[word..end]) {
                if !allow_decorations || stars == 0 {
                    return false;
                }
                decoration = true;
            }
            end = word;
        } else {
            return false;
        }
    }
}

/// Where the declarator group around a name closes: the text after the name, on its line
/// and within `DECLARATOR_WINDOW` bytes, is `)` after any array dimensions. The offset
/// just past that `)`.
fn group_close(input: &Input) -> Option<usize> {
    let mut at = input.run_within(0, DECLARATOR_WINDOW, is_blank);
    while at < DECLARATOR_WINDOW && input.at(at) == Some(b'[') {
        let inside = input.run_within(at + 1, DECLARATOR_WINDOW, |byte| {
            !matches!(byte, b']' | b'\n')
        });
        at += 1 + inside;
        if input.at(at) != Some(b']') {
            return None;
        }
        at += 1;
        at += input.run_within(at, DECLARATOR_WINDOW, is_blank);
    }
    (input.at(at) == Some(b')')).then_some(at + 1)
}

/// Whether a declarator ends right after a name: blanks on its line, within
/// `DECLARATOR_WINDOW` bytes, then `;`, `,`, `[` or `)` (typedef-qualifiers: `typedef
/// const x_t;` declares `x_t` with an implicit `int`, and `typedef void (* const k)();`
/// declares `k`).
fn declarator_ends(input: &Input) -> bool {
    let at = input.run_within(0, DECLARATOR_WINDOW, is_blank);
    matches!(input.at(at), Some(b';' | b',' | b'[' | b')'))
}

/// Whether the text after a name, on its line and within `DECLARATOR_WINDOW` bytes, is
/// `)` (after any array dimensions) and then `(` or `[`: the declarator group closes
/// before a parameter list or an array dimension.
pub(super) fn closes_before_suffix(input: &Input) -> bool {
    group_close(input).is_some_and(|at| {
        let at = at + input.run_within(at, DECLARATOR_WINDOW, is_blank);
        matches!(input.at(at), Some(b'(' | b'['))
    })
}

/// Whether a name is the only name in the parenthesised group right after the word
/// before it (parenthesised-typedefs): the text before the name, within
/// `DECLARATOR_WINDOW` bytes, is that word and then only `(`, `*`, `^`, qualifiers and
/// blanks, at least one `(` among them (`void (*`, `int ((*`, `int (`, `void (* const`);
/// and after the name the group closes on its line (`group_close`).
fn names_group(before: &[u8], input: &Input) -> bool {
    let before = &before[before.len().saturating_sub(DECLARATOR_WINDOW)..];
    let mut end = before.len();
    let mut opened = false;
    let word_before = loop {
        let Some(&byte) = end.checked_sub(1).and_then(|last| before.get(last)) else {
            break false;
        };
        match byte {
            b'(' => opened = true,
            b'*' | b'^' => {}
            _ if is_blank(byte) => {}
            // Past the group's `(`, the word before the group; inside it, a qualifier.
            _ if is_word(byte) && opened => break true,
            _ if is_word(byte) => {
                let word = before[..end]
                    .iter()
                    .rposition(|byte| !is_word(*byte))
                    .map_or(0, |last| last + 1);
                if !QUALIFIERS.contains(&&before[word..end]) {
                    break false;
                }
                end = word;
                continue;
            }
            _ => break false,
        }
        end -= 1;
    };
    word_before && group_close(input).is_some()
}
