//! The definition-bodies Correction's look-ahead: whether a function body follows a
//! function head's parameter list.
//!
//! Between the list and the body's `{` may come parenthesised declarator parts, words
//! such as attributes, comments and directive lines, but no `struct`, `union` or `enum`,
//! which start another declaration. After a list of plain parameter names there may
//! instead be up to one `;`-terminated declaration per name (K&R), with the `{` right
//! after the last `;`. For a head inside parentheses (`int (*get(void))(int) {`), only
//! what lies outside those parentheses ends the declaration or opens the body.
//!
//! The look-ahead reads at most a window past the list (64 KiB), and no read, not even
//! one through a comment or directive line, goes past it. It remembers its last scan, and
//! a head whose parameter list that scan read at the same depth reuses it, so a run of
//! heads such as an X-macro list costs linear time. Once the look-ahead has read more than
//! 16 times the file's length, each later head gets a window of 256 bytes, which keeps any
//! file linear and leaves each head's reading local. The definition-heads and
//! linkage-blocks Corrections' smaller look-aheads share the window and the work limit.

use super::input::Input;
use super::keywords;
use super::shapes::{is_word, is_word_start};

/// How far the look-ahead reads.
#[derive(Clone, Copy, Debug)]
struct Limits {
    /// Bytes read past a head's parameter list.
    window: usize,
    /// The window once the work limit is passed.
    narrow: usize,
    /// The work limit: this many times the file's length, plus `base`.
    factor: usize,
    base: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            window: 64 * 1024,
            narrow: 256,
            factor: 16,
            base: 1 << 16,
        }
    }
}

/// How the remembered scan stopped.
#[derive(Clone, Copy, Default)]
enum Stop {
    /// At `{`: whether it is at the head's depth or outside it, and whether a `;` comes
    /// right before it.
    Brace { open: bool, direct: bool },
    /// At `}`, at `=` outside parentheses, or at the end of input: no head has a body.
    #[default]
    Never,
    /// At a `;` or `,` that only the scanning head's budget made decisive.
    Budget,
    /// At the end of the scanning head's window.
    Window,
}

/// The last scan of what follows a parameter list, in source positions.
#[derive(Default)]
pub(super) struct Bodies {
    limits: Limits,
    /// The tail the scan started at, and the byte it stopped at.
    from: usize,
    to: usize,
    stop: Stop,
    /// How far the scan read: past `to` when its window ended inside a comment,
    /// directive line, literal or word, which a head with a larger window reads again.
    reach: usize,
    /// Whether the last byte read before `to`, outside comments and directives, was a
    /// `;` outside parentheses.
    direct: bool,
    /// The depth, relative to the head, of the parentheses the head sits in; what ends a
    /// declaration or opens a body counts only at this depth or outside it.
    floor: i32,
    /// The depth after each parenthesis the scan read, relative to where it started.
    depths: Vec<(usize, i32)>,
    /// What the scan read outside parentheses, up to and including where it stopped:
    /// `;`, `,`, and `struct`, `union` or `enum`.
    semicolons: Vec<usize>,
    commas: Vec<usize>,
    aggregates: Vec<usize>,
    /// Bytes read so far.
    work: usize,
    /// Macro-head recognition cannot exhaust the ordinary definition budget.
    macro_work: usize,
    /// Wrapper recognition cannot exhaust the ordinary definition budget.
    wrapper_work: usize,
    /// Whether a head's look-ahead reads only the first arm of a conditional group it
    /// opens, skipping the group's `#elif` and `#else` arms (conditional-lists).
    first_arms: bool,
    /// The groups the last scan's head opened and the scan was still inside where it
    /// stopped, and whether it skipped an arm. Either keeps the scan from being reused
    /// by a later head, which may not have opened the same groups.
    arms: Arms,
    /// Test-only direct evidence of fallback use on the unmodified input.
    #[cfg(test)]
    narrow_windows: std::cell::Cell<usize>,
}

impl Bodies {
    /// A look-ahead that reads the first arm of each conditional group it opens when
    /// `first_arms`.
    pub(super) fn new(first_arms: bool) -> Self {
        Self {
            first_arms,
            ..Self::default()
        }
    }

    /// Whether a body follows the parameter list ahead of `input`, for a head inside
    /// `parentheses` open parentheses.
    pub(super) fn follow(&mut self, input: &Input, parentheses: i32) -> bool {
        let floor = -parentheses;
        let window = self.window(input);
        let mut arms = Arms::new(self.first_arms);
        let (tail, budget, typed) = match parameters(input, window, &mut arms) {
            Ok(found) => found,
            Err(read) => {
                self.work += read;
                return false;
            }
        };
        self.work += tail;
        if parentheses == 0 && budget == 0 && !typed {
            let next = space(input, tail, window);
            let length = input.run_within(next, window, is_word);
            self.work += next.saturating_sub(tail) + length;
            // A payload such as `(2, 3)` is not a C parameter list. A written
            // type after it belongs to the next declaration, as in
            // `PRINTF_LIKE(2, 3) void logprintf(...)`, not to this head's body.
            if next + length < window && keywords::keyword_type(&input.peek_at(next, length)) {
                return false;
            }
        }
        let Some(start) = input.source_position(tail) else {
            // Pushed-back text is never remembered.
            let mut scan = Scan::fresh(input, tail, floor, false, tail + window, arms);
            let stop = scan.run(budget, 0);
            self.work += scan.read() - scan.from;
            return matches!(stop, Stop::Brace { open: true, .. })
                && scan.aggregates.is_empty()
                && scan.semicolons.is_empty();
        };
        // A scan that skipped an arm, or stopped inside a group it opened, read groups this
        // head may not have opened.
        let reusable = self.floor == floor
            && !self.arms.skipped
            && self.arms.open == 0
            && !arms.skipped
            && arms.open == 0
            && self.from < start
            && start <= self.to
            && self.depths.binary_search(&(start - 1, 0)).is_ok();
        if !reusable {
            let mut scan = Scan::fresh(input, tail, floor, false, tail + window, arms);
            let stop = scan.run(budget, 0);
            *self = Self {
                limits: self.limits,
                first_arms: self.first_arms,
                arms: Arms::new(self.first_arms),
                from: start,
                to: start,
                floor,
                work: self.work,
                macro_work: self.macro_work,
                wrapper_work: self.wrapper_work,
                #[cfg(test)]
                narrow_windows: self.narrow_windows.clone(),
                ..Self::default()
            };
            self.extend(&scan, stop);
        }
        // Where this head's window ends.
        let limit = start + window;
        loop {
            let after = |list: &[usize]| list.partition_point(|&at| at < start);
            let semicolons = &self.semicolons[after(&self.semicolons)..];
            if semicolons.len() > budget || (budget == 0 && after(&self.commas) < self.commas.len())
            {
                return false;
            }
            let read_past = matches!(self.stop, Stop::Window) && self.reach >= limit;
            if self.to >= limit || read_past {
                return false;
            }
            match self.stop {
                Stop::Never => return false,
                Stop::Brace { open, direct } => {
                    let declaration = semicolons.last().copied().unwrap_or(start);
                    let aggregate = self.aggregates.last().is_some_and(|&at| at > declaration);
                    return open && !aggregate && (semicolons.is_empty() || direct);
                }
                Stop::Budget | Stop::Window => {
                    // The last scan stopped where this head goes on.
                    let resume = self.to + usize::from(matches!(self.stop, Stop::Budget));
                    let mut scan = Scan::fresh(
                        input,
                        input.offset_of(resume),
                        floor,
                        self.direct,
                        input.offset_of(limit),
                        self.arms,
                    );
                    scan.depth = self.depth_before(resume);
                    let stop = scan.run(budget, semicolons.len());
                    self.extend(&scan, stop);
                }
            }
        }
    }

    /// Whether a balanced parenthesised list follows and `{` follows it directly, with
    /// only whitespace, comments or directive lines around (the definition-heads
    /// Correction). It reads within the same window as `follow`, and its reads count as
    /// work.
    pub(super) fn direct(&mut self, input: &Input) -> DirectHead {
        let (read, found) = direct_body(input, self.window(input), false, self.first_arms);
        self.work += read;
        found
    }

    /// Whether a conditional group follows, with only whitespace or comments before it,
    /// and its first arm starts with a parenthesised list (conditional-lists). It reads
    /// within the same window, and its reads count as work.
    pub(super) fn conditional_list(&mut self, input: &Input) -> bool {
        let window = self.window(input);
        let at = space(input, 0, window);
        let opens = at < window
            && input.at(at) == Some(b'#')
            && matches!(
                &directive_word(input, at, window)[..],
                b"if" | b"ifdef" | b"ifndef"
            );
        let list = if opens {
            arm_gap(input, at, &mut Arms::new(true), window)
        } else {
            at
        };
        self.work += list + 1;
        opens && list < window && input.at(list) == Some(b'(')
    }

    /// A generated-function head stays inside its macro's logical line. It uses
    /// the same direct-head classifier and limits as ordinary written heads, with
    /// its own work tally so recognizing calls cannot change later definitions.
    pub(super) fn macro_head(&mut self, input: &Input) -> DirectHead {
        let (read, found) =
            direct_body(input, self.window_for(input, self.macro_work), true, false);
        self.macro_work += read;
        found
    }

    /// A splice immediately after the macro name preserves formal-list adjacency.
    /// Charge these small reads to the same isolated macro budget as its prefix.
    pub(super) fn macro_header_splice(&mut self, input: &Input) -> bool {
        self.macro_work += 1;
        if input.at(0) != Some(b'\\') {
            return false;
        }
        self.macro_work += 1;
        match input.at(1) {
            Some(b'\n') => true,
            Some(b'\r') => {
                self.macro_work += 1;
                input.at(2) == Some(b'\n')
            }
            _ => false,
        }
    }

    /// The initial declaration prefix and exactly one two-name constructor list.
    /// Original prefix bytes include comments and splices; no later body prefix
    /// can restart this proof. Every scan uses the isolated macro work tally.
    pub(super) fn macro_constructor(&mut self, input: &Input, name: &[u8], from: usize) -> bool {
        if !constructor_identifier(name) {
            return false;
        }
        let name_len = name.len();
        let Some(start) = input
            .source_position(0)
            .and_then(|end| end.checked_sub(name_len))
        else {
            return false;
        };
        let Some(length) = start.checked_sub(from).filter(|&length| length <= 256) else {
            return false;
        };
        let Some((before, _)) = input.around_name(name_len, 256) else {
            return false;
        };
        let prefix = Input::new(&before[before.len() - length..]);
        let (read, valid) = constructor_prefix(&prefix, length);
        self.macro_work += read;
        if !valid {
            return false;
        }
        let (read, found) = constructor_body(input, self.window_for(input, self.macro_work));
        self.macro_work += read;
        found
    }

    /// A simple written type operand followed by a separate direct function head.
    /// This narrow recognition has its own tally and never reads past 256 bytes.
    pub(super) fn return_type_wrapper(&mut self, input: &Input) -> bool {
        let limit = self.window_for(input, self.wrapper_work).min(256);
        let (read, found) = return_type_wrapper(input, limit);
        self.wrapper_work += read;
        found
    }

    /// Whether the list contains written parameter types (or is empty), rather
    /// than only macro arguments such as `(3, 4)` or `(return_type)`.
    pub(super) fn typed_parameters(&mut self, input: &Input) -> bool {
        match parameters(input, self.window(input), &mut Arms::new(self.first_arms)) {
            Ok((read, _, typed)) => {
                self.work += read;
                typed
            }
            Err(read) => {
                self.work += read;
                false
            }
        }
    }

    /// A numeric annotation payload cannot be a written parameter list. Keep its
    /// recognition within the body look-ahead's window and shared work budget.
    pub(super) fn numeric_arguments(&mut self, input: &Input) -> bool {
        let (read, found) = numeric_arguments(input, self.window(input));
        self.work += read;
        found
    }

    /// A tag's definition may have explicit attributes before its name or body.
    /// Attribute arguments do not supply that name. The scan shares the body
    /// look-ahead's window and work limit.
    pub(super) fn attributed_tag(&mut self, input: &Input) -> Option<TagBody> {
        let mut scan = AttributeTail {
            input,
            at: 0,
            limit: self.window(input),
        };
        let result = scan.tag();
        self.work += scan.at;
        result
    }

    /// Whether a string literal on one line and then `{` follow, with only whitespace or
    /// comments around the literal: a linkage block (the linkage-blocks Correction). It
    /// reads within the same window, and its reads count as work.
    pub(super) fn linkage(&mut self, input: &Input) -> bool {
        let (read, found) = linkage_block(input, self.window(input));
        self.work += read;
        found
    }

    /// The window for the next head: the full one until the work limit is passed.
    fn window(&self, input: &Input) -> usize {
        self.window_for(input, self.work)
    }

    fn window_for(&self, input: &Input, work: usize) -> usize {
        let limit = input
            .source_len()
            .saturating_mul(self.limits.factor)
            .saturating_add(self.limits.base);
        if work > limit {
            #[cfg(test)]
            self.narrow_windows.set(self.narrow_windows.get() + 1);
            self.limits.narrow
        } else {
            self.limits.window
        }
    }

    #[cfg(test)]
    pub(super) fn cost(&self) -> super::ScanCost {
        super::ScanCost {
            work: self.work + self.macro_work + self.wrapper_work,
            narrow_windows: self.narrow_windows.get(),
            closing_refusals: 0,
        }
    }

    /// Test-only: no work limit, to tell whether the limit changed a reading.
    #[cfg(test)]
    pub(super) fn unbounded(&mut self) {
        self.limits.factor = usize::MAX / 2;
    }

    fn depth_before(&self, position: usize) -> i32 {
        let index = self.depths.partition_point(|&(at, _)| at < position);
        index.checked_sub(1).map_or(0, |index| self.depths[index].1)
    }

    fn extend(&mut self, scan: &Scan, stop: Stop) {
        let to_source = |offset: usize| scan.input.source_position(offset);
        self.work += scan.read() - scan.from;
        self.reach = to_source(scan.read()).unwrap_or(usize::MAX);
        self.depths.extend(
            scan.depths
                .iter()
                .filter_map(|&(offset, depth)| Some((to_source(offset)?, depth))),
        );
        for (mine, theirs) in [
            (&mut self.semicolons, &scan.semicolons),
            (&mut self.commas, &scan.commas),
            (&mut self.aggregates, &scan.aggregates),
        ] {
            mine.extend(theirs.iter().filter_map(|&offset| to_source(offset)));
        }
        self.to = to_source(scan.at).unwrap_or(self.to);
        self.stop = stop;
        self.direct = scan.direct;
        self.arms.open = scan.arms.open;
        self.arms.skipped |= scan.arms.skipped;
    }
}

/// Evidence from a list immediately followed by a body. A typed nested list can
/// belong to a callback parameter or to a wrapper around a whole function head;
/// the declaration prefix distinguishes those cases.
pub(super) enum DirectHead {
    None,
    Parameters,
    TypedNested,
}

pub(super) enum TagBody {
    Named,
    Anonymous,
}

struct AttributeTail<'i, 's> {
    input: &'i Input<'s>,
    at: usize,
    limit: usize,
}

impl AttributeTail<'_, '_> {
    fn skip_gap(&mut self) -> Option<u8> {
        self.at = gap(self.input, self.at, self.limit);
        (self.at < self.limit)
            .then(|| self.input.at(self.at))
            .flatten()
    }

    fn tag(&mut self) -> Option<TagBody> {
        let mut attributes = 0;
        loop {
            let byte = self.skip_gap()?;
            if byte == b'{' && attributes > 0 {
                return Some(TagBody::Anonymous);
            }
            if !is_word_start(byte) {
                return None;
            }
            let len = self.input.run_within(self.at, self.limit, is_word);
            let name = self.input.peek_at(self.at, len);
            self.at += len;
            let next = self.skip_gap()?;
            if !matches!(&name[..], b"__attribute__" | b"__attribute" | b"__declspec") {
                return (attributes > 0 && next == b'{').then_some(TagBody::Named);
            }
            if next != b'(' {
                return None;
            }
            let mut depth = 0_u32;
            loop {
                match self.skip_gap()? {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            self.at += 1;
                            break;
                        }
                    }
                    b'"' | b'\'' => {
                        self.at = literal_end(self.input, self.at, self.limit);
                        continue;
                    }
                    _ => {}
                }
                self.at += 1;
            }
            attributes += 1;
        }
    }
}

impl DirectHead {
    pub(super) fn accepts(self, prefix: bool) -> bool {
        matches!(self, Self::Parameters) || (prefix && matches!(self, Self::TypedNested))
    }
}

/// Whether `{` directly follows the balanced parenthesised list ahead, and the bytes read to
/// tell; reads at most `window` bytes. A list that wraps a whole function head
/// (`WRAP(static int f(int x))`: no comma of its own, and ending in a name with a typed
/// list) is a wrapper's, whose head the reference already reads.
fn direct_body(input: &Input, window: usize, macro_line: bool, arms: bool) -> (usize, DirectHead) {
    direct_body_from(input, 0, window, macro_line, Some(arms), true)
}

fn constructor_prefix(input: &Input, limit: usize) -> (usize, bool) {
    let (mut at, mut explicit, mut opaque) = (0, false, false);
    loop {
        at = macro_gap(input, at, limit);
        if at >= limit {
            return (at, at == limit && (explicit || opaque));
        }
        if !input.at(at).is_some_and(is_word_start) {
            return (at + 1, false);
        }
        let length = input.run_within(at, limit, is_word);
        let word = input.peek_at(at, length);
        at += length;
        if matches!(
            &word[..],
            b"static" | b"extern" | b"inline" | b"const" | b"volatile" | b"restrict"
        ) {
            continue;
        }
        if keywords::keyword_type(&word) && !opaque {
            explicit = true;
        } else if !explicit
            && !opaque
            && constructor_identifier(&word)
            && !matches!(&word[..], b"__attribute__" | b"__attribute" | b"__declspec")
        {
            opaque = true;
        } else {
            return (at, false);
        }
    }
}

fn constructor_body(input: &Input, limit: usize) -> (usize, bool) {
    let mut at = macro_gap(input, 0, limit);
    for expected in *b"(,)" {
        if at >= limit || input.at(at) != Some(expected) {
            return (at + 1, false);
        }
        at = macro_gap(input, at + 1, limit);
        if expected == b')' {
            break;
        }
        if at >= limit || !input.at(at).is_some_and(is_word_start) {
            return (at + 1, false);
        }
        let length = input.run_within(at, limit, is_word);
        let identifier = constructor_identifier(&input.peek_at(at, length));
        at += length;
        if !identifier || at >= limit {
            return (at, false);
        }
        at = macro_gap(input, at, limit);
    }
    let (read, head) = direct_body_from(input, at, limit, true, None, false);
    (read, head.accepts(true))
}

/// C99/C11 keywords absent from the reference's display table still cannot be
/// opaque type names or bare constructor operands. Display rules stay separate.
fn constructor_identifier(word: &[u8]) -> bool {
    keywords::keyword(word).is_none()
        && !matches!(
            word,
            b"inline"
                | b"restrict"
                | b"_Alignas"
                | b"_Alignof"
                | b"_Atomic"
                | b"_Bool"
                | b"_Complex"
                | b"_Generic"
                | b"_Imaginary"
                | b"_Noreturn"
                | b"_Static_assert"
                | b"_Thread_local"
        )
}

/// Whether a balanced parenthesised list from `from` is followed directly by `{`, and
/// how; with `two_lists`, a second list may stand between them, as after a macro's
/// argument list (`DEFUN ("name", Fname, ...) (Lisp_Object args) {`), and then decides.
/// With `directives`, directive lines are gaps, and with `Some(true)` only the first arm
/// of a conditional group it opens is read.
fn direct_body_from(
    input: &Input,
    from: usize,
    window: usize,
    macro_line: bool,
    directives: Option<bool>,
    two_lists: bool,
) -> (usize, DirectHead) {
    let mut arms = Arms::new(directives == Some(true));
    let mut gap = |at| {
        if macro_line {
            macro_gap(input, at, window)
        } else if directives.is_some() {
            arm_gap(input, at, &mut arms, window)
        } else {
            space(input, at, window)
        }
    };
    let mut at = gap(from);
    if at >= window || input.at(at) != Some(b'(') {
        return (at, DirectHead::None);
    }
    let (mut depth, mut list, mut second) = (0_i32, List::default(), false);
    loop {
        at = gap(at);
        if at >= window {
            return (at, DirectHead::None);
        }
        match input.at(at) {
            None => return (at, DirectHead::None),
            Some(b'(') => {
                depth += 1;
                list.open(depth);
            }
            Some(b')') => {
                list.close(depth);
                depth -= 1;
                if depth == 0 {
                    let next = gap(at + 1);
                    // A macro's argument list can be followed by the parameter list of
                    // the function it heads, as in `DEFUN ("name", Fname, ...) (args) {`:
                    // that second list and its body decide.
                    if two_lists
                        && !second
                        && !macro_line
                        && next < window
                        && input.at(next) == Some(b'(')
                    {
                        (second, list, at) = (true, List::default(), next);
                        continue;
                    }
                    let body = next < window && input.at(next) == Some(b'{');
                    let head = if !body {
                        DirectHead::None
                    } else if !list.wraps_head() {
                        DirectHead::Parameters
                    } else if list.typed {
                        DirectHead::TypedNested
                    } else {
                        DirectHead::None
                    };
                    return (next, head);
                }
            }
            Some(b'{' | b'}' | b';') => return (at, DirectHead::None),
            Some(b'\n' | b'#' | b'"' | b'\'') if macro_line => {
                return (at, DirectHead::None);
            }
            Some(b'"' | b'\'') => {
                list.other(depth);
                at = literal_end(input, at, window);
                continue;
            }
            Some(byte) if is_word_start(byte) => {
                let length = input.run_within(at, window, is_word);
                list.word(depth, &input.peek_at(at, length));
                at += length;
                continue;
            }
            Some(b',') => list.comma(depth),
            Some(b'*') => list.star(depth),
            Some(_) => list.other(depth),
        }
        at += 1;
    }
}

/// Opaque single-name operands can be K&R parameters. Written parameter names,
/// nested groups and annotations are also left to the established reading.
fn return_type_wrapper(input: &Input, limit: usize) -> (usize, bool) {
    let mut at = space(input, 0, limit);
    if at >= limit || input.at(at) != Some(b'(') {
        return (at, false);
    }
    at += 1;
    let (mut builtin, mut nonvoid, mut opaque, mut pointer) = (false, false, false, false);
    loop {
        at = space(input, at, limit);
        if at >= limit {
            return (at, false);
        }
        match input.at(at) {
            Some(b')') if nonvoid || ((builtin || opaque) && pointer) => {
                at = space(input, at + 1, limit);
                break;
            }
            Some(b'*') if builtin || opaque => pointer = true,
            Some(byte) if is_word_start(byte) => {
                let length = input.run_within(at, limit, is_word);
                if at + length >= limit {
                    return (at + length, false);
                }
                let word = input.peek_at(at, length);
                if keywords::keyword_type(&word) && !opaque && !pointer {
                    builtin = true;
                    nonvoid |= word != b"void";
                } else if !matches!(&word[..], b"const" | b"volatile" | b"restrict") {
                    if builtin || opaque || pointer || keywords::keyword(&word).is_some() {
                        return (at + length, false);
                    }
                    opaque = true;
                }
                at += length;
                continue;
            }
            _ => return (at, false),
        }
        at += 1;
    }
    if at >= limit || !input.at(at).is_some_and(is_word_start) {
        return (at, false);
    }
    let length = input.run_within(at, limit, is_word);
    if at + length >= limit || keywords::keyword(&input.peek_at(at, length)).is_some() {
        return (at + length, false);
    }
    at += length;
    let (read, head) = direct_body_from(input, at, limit, false, None, false);
    (read, matches!(head, DirectHead::Parameters))
}

/// Macro replacement text ends at an unspliced newline, including one inside
/// a comment. Closed comments and LF/CR LF splices remain declaration trivia.
pub(super) fn macro_gap(input: &Input, mut at: usize, limit: usize) -> usize {
    while at < limit {
        match (input.at(at), input.at(at + 1)) {
            (Some(b' ' | b'\t' | b'\r' | 0x0b | 0x0c), _) => at += 1,
            (Some(b'\\'), Some(b'\n')) => at += 2,
            (Some(b'\\'), Some(b'\r')) if input.at(at + 2) == Some(b'\n') => at += 3,
            (Some(b'/'), Some(b'*')) => {
                at += 2;
                loop {
                    if at >= limit {
                        return at;
                    }
                    match (input.at(at), input.at(at + 1)) {
                        (None | Some(b'\n'), _) => return at,
                        (Some(b'*'), Some(b'/')) => {
                            at += 2;
                            break;
                        }
                        (Some(b'\\'), Some(b'\n')) => at += 2,
                        (Some(b'\\'), Some(b'\r')) if input.at(at + 2) == Some(b'\n') => {
                            at += 3;
                        }
                        _ => at += 1,
                    }
                }
            }
            _ => return at,
        }
    }
    at
}

/// What a parenthesised list holds, as far as telling a wrapped function head
/// (`WRAP(static int f(int x))`) apart from a parameter list needs. Depth 1 is directly
/// inside the list; depth 2, inside a group directly inside it.
#[derive(Default)]
struct List {
    /// A `,` directly inside the list.
    comma: bool,
    /// The last thing directly inside the list was a word.
    word: bool,
    /// The list itself holds a type, before any nested list.
    typed: bool,
    /// The list ends with a group directly inside it.
    ends_in_group: bool,
    /// That group follows a word, as a head's parameter list does.
    group_named: bool,
    /// That group holds types: a keyword, `*`, or two words in a row.
    group_typed: bool,
    /// Inside that group, the last thing was a word.
    group_word: bool,
}

impl List {
    fn open(&mut self, depth: i32) {
        if depth == 2 {
            self.group_named = self.word;
            self.group_typed = false;
            self.word = false;
            self.ends_in_group = false;
        }
        self.group_word = false;
    }

    fn close(&mut self, depth: i32) {
        if depth == 2 {
            self.ends_in_group = true;
        }
        self.group_word = false;
    }

    fn word(&mut self, depth: i32, word: &[u8]) {
        if depth == 1 {
            self.typed |= self.word || keywords::keyword(word).is_some();
            self.word = true;
            self.ends_in_group = false;
        } else {
            self.group_typed |= self.group_word || keywords::keyword(word).is_some();
            self.group_word = true;
        }
    }

    fn comma(&mut self, depth: i32) {
        self.comma |= depth == 1;
        self.other(depth);
    }

    fn star(&mut self, depth: i32) {
        self.group_typed |= depth >= 2;
        self.other(depth);
    }

    fn other(&mut self, depth: i32) {
        if depth == 1 {
            self.word = false;
            self.ends_in_group = false;
        }
        self.group_word = false;
    }

    fn wraps_head(&self) -> bool {
        !self.comma && self.ends_in_group && self.group_named && self.group_typed
    }
}

/// Whether a string literal on one line and then `{` follow, and the bytes read to tell;
/// reads at most `window` bytes around the literal.
fn linkage_block(input: &Input, window: usize) -> (usize, bool) {
    let quote = space(input, 0, window);
    if quote >= window || input.at(quote) != Some(b'"') {
        return (quote, false);
    }
    let inside = input.run_within(quote + 1, window, |byte| byte != b'"' && byte != b'\n');
    let close = quote + 1 + inside;
    if close >= window || input.at(close) != Some(b'"') {
        return (close, false);
    }
    let brace = space(input, close + 1, window);
    (brace, brace < window && input.at(brace) == Some(b'{'))
}

/// Where the parameter list ahead ends, and how many `;`-terminated declarations may
/// follow it: one per name when it lists only plain names, or one per entry when each
/// entry is plain words (`len LOCALE_PARAM`, a name and a macro), otherwise none. Reads
/// at most `window` bytes; without an end there, the bytes read.
fn parameters(
    input: &Input,
    window: usize,
    arms: &mut Arms,
) -> Result<(usize, usize, bool), usize> {
    let mut at = arm_gap(input, 0, arms, window);
    if at >= window || input.at(at) != Some(b'(') {
        return Err(at);
    }
    let (mut depth, mut names, mut plain, mut expect_name) = (0_i32, 0_usize, true, true);
    let (mut typed, mut word_before) = (false, false);
    // Every entry is plain words, and how many entries there are.
    let (mut words, mut entries) = (true, 0_usize);
    loop {
        at = arm_gap(input, at, arms, window);
        if at >= window {
            return Err(at);
        }
        let Some(byte) = input.at(at) else {
            return Err(at);
        };
        match byte {
            b'(' => {
                depth += 1;
                word_before = false;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    let budget = if plain && !expect_name {
                        names
                    } else if words && !expect_name {
                        entries
                    } else {
                        0
                    };
                    return Ok((at + 1, budget, typed || (plain && names == 0)));
                }
            }
            b',' if depth == 1 => {
                words &= !expect_name;
                plain &= !expect_name;
                expect_name = true;
                word_before = false;
            }
            _ if is_word_start(byte) => {
                let length = input.run_within(at, window, is_word);
                let word = input.peek_at(at, length);
                if depth == 1 {
                    typed |= word_before || keywords::keyword(&word).is_some();
                    word_before = true;
                }
                plain &= depth == 1 && expect_name && keywords::keyword(&word).is_none();
                words &= depth == 1 && keywords::keyword(&word).is_none();
                entries += usize::from(expect_name);
                names += 1;
                expect_name = false;
                at += length;
                continue;
            }
            b'"' | b'\'' => {
                plain = false;
                words = false;
                at = literal_end(input, at, window);
                continue;
            }
            _ => {
                typed |= depth == 1 && byte == b'*';
                word_before = false;
                plain = false;
                words &= byte == b'(' || byte == b')';
            }
        }
        at += 1;
    }
}

/// One scan of what follows a parameter list, in offsets from the read position.
struct Scan<'i, 's> {
    input: &'i Input<'s>,
    from: usize,
    at: usize,
    /// How far the scan read past `at`, when its window ended inside a gap, literal or
    /// word.
    past: usize,
    /// Where the scanning head's window ends.
    until: usize,
    depth: i32,
    floor: i32,
    /// The last byte read outside comments and directives was a `;` outside parentheses.
    direct: bool,
    depths: Vec<(usize, i32)>,
    semicolons: Vec<usize>,
    commas: Vec<usize>,
    aggregates: Vec<usize>,
    /// The conditional groups the scanning head's look-ahead opened.
    arms: Arms,
}

impl<'i, 's> Scan<'i, 's> {
    fn fresh(
        input: &'i Input<'s>,
        at: usize,
        floor: i32,
        direct: bool,
        until: usize,
        arms: Arms,
    ) -> Self {
        Self {
            input,
            from: at,
            at,
            past: at,
            until,
            depth: 0,
            floor,
            direct,
            depths: Vec::new(),
            semicolons: Vec::new(),
            commas: Vec::new(),
            aggregates: Vec::new(),
            arms,
        }
    }

    /// The end of what the scan read.
    fn read(&self) -> usize {
        self.at.max(self.past)
    }

    /// Reads up to the body or to what rules it out, with `declarations` of `budget`
    /// already read; leaves `at` on the byte it stopped at, or, when the window ends
    /// inside a gap, literal or word, on its start.
    fn run(&mut self, budget: usize, mut declarations: usize) -> Stop {
        loop {
            let end = arm_gap(self.input, self.at, &mut self.arms, self.until);
            if end >= self.until {
                self.past = end;
                return Stop::Window;
            }
            self.at = end;
            let Some(byte) = self.input.at(self.at) else {
                return Stop::Never;
            };
            let direct = std::mem::replace(&mut self.direct, false);
            let outside = self.depth <= self.floor;
            match byte {
                b'{' => {
                    return Stop::Brace {
                        open: outside,
                        direct,
                    };
                }
                b'}' => return Stop::Never,
                b'(' | b')' => {
                    self.depth += if byte == b'(' { 1 } else { -1 };
                    self.depths.push((self.at, self.depth));
                }
                b';' if outside => {
                    self.semicolons.push(self.at);
                    self.direct = true;
                    if declarations == budget {
                        return Stop::Budget;
                    }
                    declarations += 1;
                }
                b'=' if outside => return Stop::Never,
                b',' if outside => {
                    self.commas.push(self.at);
                    if budget == 0 {
                        return Stop::Budget;
                    }
                }
                b'"' | b'\'' => {
                    let end = literal_end(self.input, self.at, self.until);
                    if end >= self.until {
                        self.past = end;
                        return Stop::Window;
                    }
                    self.at = end;
                    continue;
                }
                _ if is_word_start(byte) => {
                    let length = self.input.run_within(self.at, self.until, is_word);
                    if self.at + length >= self.until {
                        self.past = self.at + length;
                        return Stop::Window;
                    }
                    let word = self.input.peek_at(self.at, length);
                    if outside && matches!(&word[..], b"struct" | b"union" | b"enum") {
                        self.aggregates.push(self.at);
                    }
                    // A macro word such as `PARAM_DECL` may stand between the last K&R
                    // declaration and the body; a keyword starts another declaration.
                    self.direct = direct && outside && keywords::keyword(&word).is_none();
                    self.at += length;
                    continue;
                }
                _ => {}
            }
            self.at += 1;
        }
    }
}

/// A complete, nonempty list of decimal operands, with optional fractional parts.
/// Other payloads keep their existing reading, including typed and K&R lists.
fn numeric_arguments(input: &Input, limit: usize) -> (usize, bool) {
    let mut at = space(input, 0, limit);
    if at >= limit || input.at(at) != Some(b'(') {
        return (at, false);
    }
    at += 1;
    loop {
        at = space(input, at, limit);
        let digits = input.run_within(at, limit, |byte| byte.is_ascii_digit());
        if digits == 0 {
            return (at, false);
        }
        at += digits;
        if at < limit && input.at(at) == Some(b'.') {
            at += 1;
            let fraction = input.run_within(at, limit, |byte| byte.is_ascii_digit());
            if fraction == 0 {
                return (at, false);
            }
            at += fraction;
        }
        at = space(input, at, limit);
        if at >= limit {
            return (at, false);
        }
        match input.at(at) {
            Some(b')') => return (at + 1, true),
            Some(b',') => at += 1,
            _ => return (at, false),
        }
    }
}

/// Past whitespace, comments and directive lines from `at`, reading no further than
/// `limit`: a result at or past `limit` means the limit was reached.
fn gap(input: &Input, at: usize, limit: usize) -> usize {
    skip(input, at, true, limit)
}

/// Past whitespace and comments from `at`, reading no further than `limit`.
pub(super) fn space(input: &Input, at: usize, limit: usize) -> usize {
    skip(input, at, false, limit)
}

/// The conditional groups a head's look-ahead has opened (conditional-lists).
#[derive(Clone, Copy, Default)]
struct Arms {
    /// Only the first arm of a group it opens is read.
    first: bool,
    /// How many of those groups it is inside.
    open: usize,
    /// It skipped a later arm.
    skipped: bool,
}

impl Arms {
    fn new(first: bool) -> Self {
        Self {
            first,
            ..Self::default()
        }
    }
}

/// Past whitespace, comments and directive lines from `at`, as `gap`; when `arms` reads
/// first arms, an `#elif` or `#else` line of a group the look-ahead opened ends the arm
/// being read, and the rest of the group up to its `#endif` is skipped too. Groups that
/// were open before the look-ahead keep all their arms.
fn arm_gap(input: &Input, at: usize, arms: &mut Arms, limit: usize) -> usize {
    if !arms.first {
        return gap(input, at, limit);
    }
    let mut at = at;
    loop {
        at = space(input, at, limit);
        if at >= limit || input.at(at) != Some(b'#') {
            return at;
        }
        at = match &directive_word(input, at, limit)[..] {
            b"if" | b"ifdef" | b"ifndef" => {
                arms.open += 1;
                line_end(input, at, limit)
            }
            b"endif" => {
                arms.open = arms.open.saturating_sub(1);
                line_end(input, at, limit)
            }
            b"elif" | b"else" if arms.open > 0 => {
                arms.open -= 1;
                arms.skipped = true;
                group_end(input, at, limit)
            }
            _ => line_end(input, at, limit),
        };
    }
}

/// The word of the directive whose `#` is at `at`.
pub(super) fn directive_word(input: &Input, at: usize, limit: usize) -> Vec<u8> {
    let word = at + 1 + input.run_within(at + 1, limit, |byte| byte == b' ' || byte == b'\t');
    let length = input.run_within(word, limit, |byte| byte.is_ascii_alphabetic());
    input.peek_at(word, length)
}

/// The end of the `#endif` line closing the group whose `#elif` or `#else` is at `at`,
/// counting groups nested in its later arms, or `limit` if that comes first.
fn group_end(input: &Input, mut at: usize, limit: usize) -> usize {
    let mut nested = 0_usize;
    loop {
        at = line_end(input, at, limit);
        if at >= limit || input.at(at).is_none() {
            return at.max(limit);
        }
        at += 1;
        at += input.run_within(at, limit, |byte| byte == b' ' || byte == b'\t');
        if input.at(at) != Some(b'#') {
            continue;
        }
        match &directive_word(input, at, limit)[..] {
            b"if" | b"ifdef" | b"ifndef" => nested += 1,
            b"endif" if nested == 0 => return line_end(input, at, limit),
            b"endif" => nested -= 1,
            _ => {}
        }
    }
}

fn skip(input: &Input, mut at: usize, directives: bool, limit: usize) -> usize {
    while at < limit {
        match (input.at(at), input.at(at + 1)) {
            (Some(b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c), _) => at += 1,
            (Some(b'\\'), Some(b'\n')) => at += 2,
            (Some(b'\\'), Some(b'\r')) if input.at(at + 2) == Some(b'\n') => at += 3,
            (Some(b'/'), Some(b'*')) => {
                at += 2;
                while at < limit
                    && input.at(at).is_some()
                    && !(input.at(at) == Some(b'*') && input.at(at + 1) == Some(b'/'))
                {
                    at += 1;
                }
                at += 2;
            }
            (Some(b'/'), Some(b'/')) => at = line_end(input, at, limit),
            (Some(b'#'), _) if directives => at = line_end(input, at, limit),
            _ => return at,
        }
    }
    at
}

/// The line end that ends the line holding `at`, past backslash continuations, or
/// `limit` if that comes first.
pub(super) fn line_end(input: &Input, mut at: usize, limit: usize) -> usize {
    while at < limit {
        let Some(byte) = input.at(at) else {
            break;
        };
        if byte == b'\n' {
            break;
        }
        at += match (byte, input.at(at + 1), input.at(at + 2)) {
            (b'\\', Some(b'\n'), _) => 2,
            (b'\\', Some(b'\r'), Some(b'\n')) => 3,
            _ => 1,
        };
    }
    at
}

/// Past the string or character literal that starts at `at`, or past its line, or at
/// `limit` if that comes first.
pub(super) fn literal_end(input: &Input, at: usize, limit: usize) -> usize {
    let closer = input.at(at);
    let mut end = at + 1;
    while end < limit {
        let Some(byte) = input.at(end) else {
            break;
        };
        end += 1;
        if Some(byte) == closer || byte == b'\n' {
            break;
        }
        if byte == b'\\' {
            end += 1;
        }
    }
    end
}

#[cfg(test)]
mod tests;
