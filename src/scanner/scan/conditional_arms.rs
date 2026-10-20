//! Saved conditional arms, their selective restoration and delayed joins.
//! Directive recognition and source consumption stay in Scan (reading-c.md §6).

use super::{CallContext, CallPrefix, Declaration, Nesting, PendingHead, TypedefPrefix};
use crate::scanner::{Corrections, input::Input, shapes::LookAhead, tails::Tail};

/// Every arm starts at its group's opening brace depth. Closure leaves the deepest
/// arm's depth behind, without restoring the general parenthesis depth.
pub(super) struct ConditionalArms {
    groups: Vec<Group>,
    /// Opening clears this; else sets it even unmatched; elif leaves it unchanged.
    had_else: bool,
    /// A following #if continues the group whose #endif was just read.
    continues: bool,
    heads: bool,
    tails: bool,
    typedefs: bool,
    calls: bool,
    /// An arm that closed more braces than it opened ends its group, and so does one
    /// whose following arm would (closing-arms).
    closing: bool,
    /// Bytes the following-arm look-ahead has read, against its limit.
    looked: usize,
    /// The group just closed ended so (closing-arms): the next opens after it.
    split: bool,
    /// The closing-arms look-ahead's work limit, this many times the file's length.
    factor: usize,
    /// Test-only: how often that limit stopped the look-ahead.
    #[cfg(test)]
    pub(super) refusals: usize,
    /// Each arm at file scope, or one that leaves a head pending, starts at the group's
    /// parenthesis depth, so each arm's head opens its list alike (conditional-lists).
    lists: bool,
}

impl ConditionalArms {
    pub(super) fn new(corrections: Corrections) -> Self {
        Self {
            groups: Vec::new(),
            had_else: false,
            continues: false,
            heads: corrections.definition_bodies,
            tails: corrections.bare_directive_words || corrections.opaque_member_tails,
            typedefs: corrections.typedef_decorations,
            calls: corrections.dereferenced_calls,
            closing: corrections.closing_arms,
            looked: 0,
            split: false,
            factor: 16,
            #[cfg(test)]
            refusals: 0,
            lists: corrections.conditional_lists,
        }
    }

    pub(super) fn open(&mut self, nesting: &mut Nesting, declaration: &mut Declaration) {
        self.had_else = false;
        let after_split = std::mem::replace(&mut self.split, false);
        if self.continues {
            self.continues = false;
            self.next_arm(nesting, declaration);
        } else {
            self.groups.push(Group {
                after_split,
                first_arm: None,
                calls_before: nesting.calls,
                calls_join: None,
                depth_before: nesting.depth,
                parentheses_before: nesting.parentheses,
                deepest_arm: 0,
                head_before: nesting.head_pending,
                function_before: nesting.function,
                type_prefix_before: declaration.type_prefix,
                typedef_prefix_before: declaration.typedef_prefix,
                typedef_prefix_join: None,
                tail_before: declaration.tail,
                tail_join: None,
                structure_before: nesting.file_structure,
                structure_join: None,
                has_else: false,
            });
        }
    }

    /// An elif changes arms without marking an else, even for unmatched directives.
    pub(super) fn alternate(
        &mut self,
        nesting: &mut Nesting,
        declaration: &mut Declaration,
        is_else: bool,
    ) {
        if is_else {
            self.had_else = true;
            if let Some(group) = self.groups.last_mut() {
                group.has_else = true;
            }
        }
        self.next_arm(nesting, declaration);
    }

    /// Called after the endif tail is consumed, before Scan consumes its newline.
    pub(super) fn close(
        &mut self,
        nesting: &mut Nesting,
        declaration: &mut Declaration,
        input: &Input,
        look: &mut LookAhead,
    ) {
        let continued = look.continued_conditional(input);
        let closed = look.endif_gaps(input);
        let continues = match (continued, closed) {
            (Some(continued), Some(closed)) => continued >= closed,
            (Some(_), None) => true,
            _ => false,
        };
        let mut sequential = || {
            let Some(group) = self.groups.last() else {
                return false;
            };
            if nesting.depth < group.depth_before {
                return true;
            }
            // The look-ahead stops reading once it has read 16 times the file's length.
            let limit = self
                .factor
                .saturating_mul(input.source_len())
                .saturating_add(1 << 16);
            if self.looked > limit {
                #[cfg(test)]
                {
                    self.refusals += 1;
                }
                return false;
            }
            let (read, closes) = arm_closes(input, continued.unwrap_or(0), ARM_WINDOW);
            self.looked += read;
            closes
        };
        let split = continues && !self.had_else && self.closing && sequential();
        if continues && !self.had_else && !split {
            self.continues = true;
        } else if let Some(mut group) = self.groups.pop() {
            self.split = split;
            if self.calls {
                group.join_calls(nesting.calls);
                if !group.has_else {
                    group.join_calls(group.calls_before);
                }
                nesting.calls = group.calls_join.expect("current arm joined");
            }
            // A group read after the one it closes a brace of keeps its first arm's
            // depth, as with every first arm taken, rather than its deepest arm's.
            nesting.depth = match group.first_arm {
                Some(first) if group.after_split => first,
                _ => nesting.depth.max(group.deepest_arm),
            };
            if self.typedefs {
                group.join_typedef_prefix(declaration.typedef_prefix);
                if !group.has_else {
                    group.join_typedef_prefix(group.typedef_prefix_before);
                }
                declaration.typedef_prefix = group.typedef_prefix_join.expect("current arm joined");
            }
            if self.tails {
                group.join_tail(declaration.tail, nesting.file_structure);
                if !group.has_else {
                    group.join_tail(group.tail_before, group.structure_before);
                }
                declaration.tail = group.tail_join.expect("current arm joined");
                nesting.file_structure = group.structure_join.expect("current arm joined");
            }
        }
    }

    /// Test-only: no work limit, to tell whether the limit changed a reading.
    #[cfg(test)]
    pub(super) fn unbounded(&mut self) {
        self.factor = usize::MAX / 2;
    }

    /// Whether the innermost group opened between a function's head and its body, at
    /// `depth`, the depth where function bodies open (alternative-bodies).
    pub(super) fn after_head(&self, depth: i32) -> bool {
        self.groups
            .last()
            .is_some_and(|group| group.function_before && group.depth_before == depth)
    }

    fn next_arm(&mut self, nesting: &mut Nesting, declaration: &mut Declaration) {
        if let Some(group) = self.groups.last_mut() {
            if self.calls {
                group.join_calls(nesting.calls);
                nesting.calls = group.calls_before;
            }
            if self.typedefs {
                group.join_typedef_prefix(declaration.typedef_prefix);
                declaration.typedef_prefix = group.typedef_prefix_before;
            }
            if self.tails {
                group.join_tail(declaration.tail, nesting.file_structure);
                declaration.tail = group.tail_before;
                nesting.file_structure = group.structure_before;
            }
            group.deepest_arm = group.deepest_arm.max(nesting.depth);
            group.first_arm.get_or_insert(nesting.depth);
            nesting.depth = group.depth_before;
            // At file scope each arm starts at the group's parenthesis depth, so an
            // alternative head whose list goes on after the group reads as the first.
            if self.lists && nesting.depth == 0 && !nesting.function {
                nesting.parentheses = group.parentheses_before;
            }
            // Alternative heads may share a return type before the #if. An
            // annotation conditional opened after a head keeps that head.
            if self.heads && nesting.head_pending.is_some() {
                nesting.head_pending = group.head_before;
                nesting.function = group.function_before;
                declaration.type_prefix = group.type_prefix_before;
                if self.lists {
                    nesting.parentheses = group.parentheses_before;
                }
            }
        }
    }
}

/// How far the closing-arms look-ahead reads into a following arm.
const ARM_WINDOW: usize = 64 * 1024;

/// How far the alternative-bodies look-ahead reads past a body's `}`.
const BODY_WINDOW: usize = 4 * 1024;

/// Whether an `#elif` or `#else` line follows the `}` at the start of `input`, and then
/// `{`, with only whitespace and comments around the directive line: the next arm opens
/// another body for the same head (alternative-bodies).
pub(super) fn alternative_body(input: &Input) -> bool {
    use crate::scanner::bodies::{directive_word, line_end, space};
    let at = space(input, 1, BODY_WINDOW);
    if at >= BODY_WINDOW
        || input.at(at) != Some(b'#')
        || !matches!(
            &directive_word(input, at, BODY_WINDOW)[..],
            b"elif" | b"else"
        )
    {
        return false;
    }
    let brace = space(input, line_end(input, at, BODY_WINDOW), BODY_WINDOW);
    brace < BODY_WINDOW && input.at(brace) == Some(b'{')
}

/// Whether the arm whose `#if` condition starts at `from` closes a brace it did not
/// open, reading its lines up to its own `#elif`, `#else` or `#endif`, and in a group
/// nested in it only the first arm, outside comments and literals; and the bytes read.
/// An arm that runs past `window` bytes, or to the end of input, counts as not closing.
fn arm_closes(input: &Input, from: usize, window: usize) -> (usize, bool) {
    let mut at = from;
    let (mut balance, mut nested, mut skipping) = (0_i32, Vec::<bool>::new(), 0_usize);
    let mut line_start = false;
    // The rest of the `#if` line, with its continuations.
    let directive_end = |mut at: usize| {
        while let Some(byte) = input.at(at) {
            if byte == b'\\' && input.at(at + 1) == Some(b'\n') {
                at += 2;
            } else if byte == b'\n' {
                break;
            } else {
                at += 1;
            }
            if at >= window {
                break;
            }
        }
        at
    };
    at = directive_end(at);
    while at < window {
        let Some(byte) = input.at(at) else {
            return (at, false);
        };
        if byte == b'\n' {
            (at, line_start) = (at + 1, true);
            continue;
        }
        if line_start && (byte == b' ' || byte == b'\t') {
            at += 1;
            continue;
        }
        if line_start && byte == b'#' {
            let word_at = at + 1 + input.run_within(at + 1, window, |b| b == b' ' || b == b'\t');
            let length = input.run_within(word_at, window, |b| b.is_ascii_alphabetic());
            match &input.peek_at(word_at, length)[..] {
                b"if" | b"ifdef" | b"ifndef" => nested.push(false),
                b"elif" | b"else" => match nested.last_mut() {
                    None => return (at, false),
                    Some(other) => {
                        if !*other {
                            *other = true;
                            skipping += 1;
                        }
                    }
                },
                b"endif" => match nested.pop() {
                    None => return (at, false),
                    Some(other) => skipping -= usize::from(other),
                },
                _ => {}
            }
            at = directive_end(word_at + length);
            line_start = false;
            continue;
        }
        line_start = false;
        match byte {
            b'/' if input.at(at + 1) == Some(b'*') => {
                at += 2;
                while at < window && input.at(at).is_some() {
                    if input.at(at) == Some(b'*') && input.at(at + 1) == Some(b'/') {
                        at += 2;
                        break;
                    }
                    at += 1;
                }
            }
            b'/' if input.at(at + 1) == Some(b'/') => {
                at += 2 + input.run_within(at + 2, window, |b| b != b'\n');
            }
            b'"' | b'\'' => {
                at += 1;
                while let Some(next) = input.at(at) {
                    if at >= window || next == b'\n' {
                        break;
                    }
                    at += if next == b'\\' { 2 } else { 1 };
                    if next == byte {
                        break;
                    }
                }
            }
            b'{' => {
                balance += i32::from(skipping == 0);
                at += 1;
            }
            b'}' => {
                if skipping == 0 {
                    balance -= 1;
                    if balance < 0 {
                        return (at, true);
                    }
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    (at, false)
}

/// Only the state each conditional reading restores or joins is saved here.
struct Group {
    /// It opened right after a group that closing-arms ended, and its first arm's depth.
    after_split: bool,
    first_arm: Option<i32>,
    calls_before: CallContext,
    calls_join: Option<CallContext>,
    depth_before: i32,
    parentheses_before: i32,
    deepest_arm: i32,
    head_before: Option<PendingHead>,
    function_before: bool,
    type_prefix_before: bool,
    typedef_prefix_before: TypedefPrefix,
    typedef_prefix_join: Option<TypedefPrefix>,
    tail_before: Tail,
    tail_join: Option<Tail>,
    structure_before: bool,
    structure_join: Option<bool>,
    has_else: bool,
}

impl Group {
    fn join_calls(&mut self, calls: CallContext) {
        self.calls_join = Some(
            self.calls_join
                .map_or(calls, |previous| previous.merge(calls)),
        );
    }

    fn join_typedef_prefix(&mut self, prefix: TypedefPrefix) {
        self.typedef_prefix_join = Some(
            self.typedef_prefix_join
                .map_or(prefix, |previous| previous.merge(prefix)),
        );
    }

    fn join_tail(&mut self, tail: Tail, structure: bool) {
        self.tail_join = Some(self.tail_join.map_or(tail, |previous| previous.merge(tail)));
        self.structure_join = Some(self.structure_join.unwrap_or(true) && structure);
    }
}

impl CallContext {
    fn merge(self, other: Self) -> Self {
        Self {
            prefix: if self.prefix == other.prefix {
                self.prefix
            } else if matches!(self.prefix, CallPrefix::Boundary | CallPrefix::Expression)
                && matches!(other.prefix, CallPrefix::Boundary | CallPrefix::Expression)
            {
                CallPrefix::Expression
            } else {
                CallPrefix::Other
            },
            closing: if self.closing == other.closing {
                self.closing
            } else {
                None
            },
            declaration: self.declaration.into_iter().chain(other.declaration).min(),
            cast: if self.cast == other.cast {
                self.cast
            } else {
                None
            },
            initializer: if self.initializer == other.initializer {
                self.initializer
            } else {
                None
            },
            arrays: self.arrays.max(other.arrays),
        }
    }
}

impl TypedefPrefix {
    fn merge(self, other: Self) -> Self {
        Self {
            words: self.words && other.words,
            pointer: self.pointer || other.pointer,
        }
    }
}
