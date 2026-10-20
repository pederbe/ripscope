//! Ripscope's own C scanner (ADR 0004): one file's bytes in, its line records out.
//!
//! The baseline reads C as the pinned reference cscope does; docs/architecture/
//! reading-c.md specifies how, and tests/fixtures/scanner/ holds the evidence.

mod assignments;
mod bodies;
mod input;
mod keywords;
mod line;
mod mutations;
mod paths;
mod render;
mod scan;
mod shapes;
mod tails;
#[cfg(test)]
mod tests;

use crate::unit::Mark;

#[cfg(test)]
pub(crate) use render::reference_view;

/// How a file is read, chosen by its last extension (reading-c.md §10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    C,
    Lex,
    Yacc,
    StateDescription,
    Breakpoints,
}

impl Mode {
    pub(crate) fn for_name(name: &[u8]) -> Self {
        let extension = name
            .iter()
            .rposition(|byte| *byte == b'.')
            .map_or(&[][..], |dot| &name[dot + 1..]);
        match extension {
            b"l" => Self::Lex,
            b"y" => Self::Yacc,
            b"sd" => Self::StateDescription,
            b"bp" => Self::Breakpoints,
            _ => Self::C,
        }
    }
}

/// One stored line: its number, its displayed text and the names on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub line: u32,
    pub text: Vec<u8>,
    pub symbols: Vec<Symbol>,
    /// A macro definition ends after this record.
    pub define_end_after: bool,
}

/// A name on a record: `text[at..at + len]`. Function and inline macro ends have no name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Symbol {
    pub mark: Mark,
    pub at: usize,
    pub len: usize,
    /// Field 9's test finds an assignment after the name (reading-c.md §7).
    pub assigned: bool,
}

/// The scanner's version, part of the analyzer identity an index stores: a different
/// version analyses every file again. Bump it whenever the records any input produces
/// change, read as the reference reads it or with the enabled Corrections; the probe
/// digest test in `tests.rs` catches changes the probes can see (a new probe changes the
/// digest too, without needing a new version).
pub(crate) const VERSION: u32 = 38;

/// A path in the canonical form the reference stores file names and include operands in
/// (reading-c.md §6).
pub(crate) fn canonical_path(path: &[u8]) -> Vec<u8> {
    paths::canonical(path)
}

/// Declares the Corrections, each once, with its switch's documentation, its field and
/// its name: the struct, the full set, the names in their fixed order and the switches.
macro_rules! corrections {
    ($($(#[$doc:meta])* $field:ident = $name:literal,)*) => {
        /// Corrections: rules by which ripscope reads a construct differently from the
        /// reference because the reference's reading is wrong (ADR 0002). Each can be
        /// switched on alone for measurement, though several depend on each other
        /// (reading-c.md §12). A clear fix is enabled once measured and reviewed. One that
        /// trades a correct reading for another stays off. All are off in the baseline, the
        /// reference reading the compass compares with cscope; the index is built with the
        /// enabled ones (`ENABLED`).
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub(crate) struct Corrections {
            $($(#[$doc])* pub $field: bool,)*
        }

        impl Corrections {
            /// Every Correction.
            const ALL: Self = Self { $($field: true,)* };

            /// Each Correction's name, in a fixed order.
            const NAMES: &'static [&'static str] = &[$($name,)*];

            /// Each Correction's switch, in the order of `NAMES`.
            fn switches(&mut self) -> Vec<&mut bool> {
                vec![$(&mut self.$field,)*]
            }
        }
    };
}

corrections! {
    /// `extern "C" {` opens a linkage block, not an extern declaration: its braces do
    /// not nest, so what it holds is read as at file scope. The reference instead keeps
    /// every member, enumerator and global in such a block (and up to the next
    /// file-scope `;` after it) from being recorded as a definition.
    linkage_blocks = "linkage-blocks",
    /// A function definition has a body (see `bodies`): a name with a parameter list,
    /// outside any directive, is a definition only when its body follows. The reference
    /// also takes declarations such as `T (*name)(int);`, `MACRO(T) name(int);` and
    /// `f(int) ATTRIBUTE;` as definitions, and reads what follows as inside them. A head
    /// without a body is a function declaration, whose words up to its `;`, `,` or `{`
    /// declare nothing; a head inside a definition's parameter list is a parameter's type.
    /// An attribute is always a macro among the declaration's specifiers, whose form
    /// ends with its list, including before a function body. A head without a body is
    /// also a specifier when it begins its declaration in a linkage block
    /// (`PyAPI_DATA(int) name;`), where the reference, one brace deep, reads it so too
    /// once its `extern` state has ended. An initial specifier with only decimal
    /// numeric arguments leaves room for another specifier (`DEPRECATED(3.12)
    /// API_DATA(int) name;`). Typed, named and empty initial lists keep their reading.
    /// A proven pointer declarator's complete
    /// parameter list can start on a later line or after a closed block comment,
    /// within the 256-byte declarator window. Its parameters and suffix annotations
    /// declare nothing, until the comma or semicolon. Inline aggregate bodies stay unchanged.
    /// At top level, a list that is neither typed parameters nor K&R names cannot
    /// borrow a later body across a written built-in type: `FORMAT(2, 3) void f(...)`
    /// leaves `FORMAT` as an occurrence. Comments may separate the list and type,
    /// but directives and unknown type names keep their existing reading.
    definition_bodies = "definition-bodies",
    /// A directive's continuation lines are read as its first line is: nothing on them
    /// is a definition. The reference reads them as code, so `(X < 5)` on the second
    /// line of an `#if` defines a global `X`, and `defined(B)` there can open a function.
    /// The same holds for the rest of a directive's own line where the reference reads
    /// code: after an `#include` operand (`#include "a.h" int x;`) or a `#` in mid-line.
    directive_continuations = "directive-continuations",
    /// A backslash before CR LF continues the line, as one before LF does. The reference
    /// takes the CR for a blank after the backslash, so in a CRLF file every multi-line
    /// macro ends on its first line and the rest is read as code at file scope.
    crlf_continuations = "crlf-continuations",
    /// `#if` and `#elif` directly followed by punctuation (`#if(A)`, `#elif!B`) are
    /// conditionals. The reference ignores them, which shifts its stack of open
    /// conditionals. Unlike a spaced `#if` right after an `#endif`, which the reference
    /// continues as a pseudo-`#elif`, an unspaced one opens a new group.
    unspaced_conditionals = "unspaced-conditionals",
    /// A macro definition is read on its own and changes nothing after it: its braces,
    /// and any `typedef`, `extern` or initializer it starts, end with it. The reference
    /// carries them on, so `#define A typedef int T[1]` makes every later name a typedef
    /// up to the next `;`, and `#define BEGIN {` puts the rest of the file inside a block.
    /// An unfinished quoted token also ends at an unspliced newline, restoring the
    /// enclosing declaration or function even when the replacement text is malformed.
    macro_bodies = "macro-bodies",
    /// An `=` inside an initializer's braces (`.b = 2` in `{ .a = 1, .b = 2 }`) leaves
    /// the initializer open, so the names after it define nothing. The reference takes it
    /// for a new initializer, and the next `,` then makes `.b` and later entries global
    /// definitions.
    initializer_entries = "initializer-entries",
    /// A name at file scope followed by a parenthesised list and directly by `{` is a
    /// function definition, also where the reference's definition shape rejects the list
    /// (a macro such as `STACK_OF(X)` in a parameter, or a line break inside a
    /// function-pointer parameter), unless a definition is open. The reference then
    /// reads the body at file scope, so its locals become global definitions and its
    /// calls are lost. After a declaration that the reference takes for a definition, a
    /// definition stays open until the next body ends; definition-bodies closes it.
    definition_heads = "definition-heads",
    /// A line comment on a macro definition's line ends with the line, and so does the
    /// macro. The reference reads the line end as part of the comment, so the macro runs
    /// on through the next line: after `#define ONE 1 // one`, `int after;` is inside the
    /// macro, defines nothing and takes the macro's context.
    macro_line_comments = "macro-line-comments",
    /// The name of a pointer declarator, to a function (`T (*name)(int);`,
    /// `T (*name[4])(int);`, `T (* const name)(int);`) or to an array (`T (*name)[4];`),
    /// is a global at file scope and a member in an aggregate, wherever a bare name there
    /// would be one. The reference leaves the name plain where the word right before the
    /// group opens a form: in an aggregate it reads `T (` as a call when `T` is not a
    /// keyword (`uint64_t (*hash)(const void *key);` defines the parameter `key` instead),
    /// and at file scope it reads `T (` mostly as a function head (`size_t (*hook)(void);`
    /// defines `size_t` as a function), and `int (` before a parameter list as a keyword
    /// head. It needs definition-bodies, under which such a head without a body opens no
    /// function and its parameters define nothing. Decoration words before an explicit
    /// pointer operator (`T (WINAPI *name)(int);`) also count when the suffix is visibly
    /// a parameter list: empty, starting with a keyword type, or containing an outer
    /// comma within the look window. These groups require aggregate or actual file scope.
    /// A parameter list on a later line also counts with definition-bodies when the
    /// complete list is proven within the window, without an inline aggregate body.
    pointer_declarators = "pointer-declarators",
    /// In a typedef whose type is a keyword right before a parenthesised declarator,
    /// `typedef void (*f)();`, `typedef int (name)();` or `typedef int (*A)[4];`, the
    /// keyword is the type and the declarator's name the typedef name. The reference reads
    /// such a keyword as a call head (where the definition shape rejects what follows the
    /// group) whose form ends without counting as the type, so the next name takes the
    /// type's place: the declarator's name becomes a global. The keyword counts only when
    /// the group, on the name's line, holds nothing but `(`, `*`, `^`, qualifiers and
    /// blanks before the name and closes after it (array dimensions allowed), each within
    /// 256 bytes of the name; a word before the name (`( __interrupt __far * pxISR )`,
    /// `(RENAME(name))`), a name on a later line and a nested declarator keep the
    /// reference's reading.
    parenthesised_typedefs = "parenthesised-typedefs",
    /// A qualifier (`const`, `volatile`) right after
    /// `typedef` is not its type, so the type name after it (`foo_t` in
    /// `typedef const foo_t bar_t;`, `png_struct` in `typedef const png_struct *p;`) is a
    /// plain name, not a typedef name, even where the reference's shape would make it a
    /// global (`typedef const T *(*f)(void);`, unlike `typedef T *(*f)(void);`). That name
    /// is the declarator only where `;`, `,`, `[` or `)` follows it on its line
    /// (`typedef const x_t;`, an implicit `int`), and a body after the qualifier is the
    /// type (`typedef const struct tag {} name;`). The reference takes any keyword for the
    /// type, and defines every later name as a typedef. `typedef const sds (alias);`
    /// defines only `alias`.
    typedef_qualifiers = "typedef-qualifiers",
    /// An `extern` still pending when a function body closes at file scope ends there: the
    /// one before an `extern` function definition, or an unterminated one before the
    /// function (no body can sit inside an `extern` declaration). The reference keeps its
    /// `extern` state until a `;` at the depth of the `extern`, and the body's own `;` are
    /// deeper, so after `extern int f(int a) { g(); }` the next declaration, up to the next
    /// file-scope `;`, defines nothing (`int x;`). macro-bodies
    /// relies on it after extern definitions: a `;` in a later macro no longer ends that
    /// state, so without this Correction the loss runs on to a `;` outside macros (the
    /// members of Ruby's `st_str_end_key` in `regparse.c`). Alone it also lets the
    /// reference's other misreadings of what follows show where the `extern` state hid
    /// them (a body read at file scope, designated initializer entries), which the other
    /// Corrections stop.
    extern_definitions = "extern-definitions",
    /// Names inside a recognised function typedef's parameter list are occurrences,
    /// not new typedefs. This covers a direct list starting with a keyword type (after
    /// any qualifiers), parenthesised names after a keyword type, and parenthesised
    /// or decorated pointer groups with a complete, visibly typed parameter suffix.
    /// Tagged return types are occurrences when the pointer declarator is proved.
    /// Unknown single-parameter groups stay unchanged: `TYPEOF(*p) (API alias)`
    /// might declare `alias`, so its group cannot safely be called a parameter list.
    typedef_parameters = "typedef-parameters",
    /// Explicit GNU and Microsoft attribute introducers and labels are names, not
    /// function heads or calls. Calls nested in their expression arguments retain
    /// their written function or macro context; a prefix attribute is at file scope.
    attribute_expressions = "attribute-expressions",
    /// A closed multiline comment after `#else` or `#endif` remains a comment.
    /// The reference swallows its opening line and reads the remaining lines as
    /// code. Only comment/whitespace tails ending within 64 KiB are corrected;
    /// actual code on a comment's closing line, or a line splice inside the
    /// comment, keeps the reference reading.
    conditional_comments = "conditional-comments",
    /// Preprocessing macro names can spell C keywords (`#define const`). They
    /// remain macro definitions instead of changing C declaration state.
    keyword_macros = "keyword-macros",
    /// A lone opaque word before a directive is an occurrence, not a global.
    /// Written declaration prefixes through conditional arms are preserved, as are
    /// two-word sequences that can be a type and a real declarator.
    bare_directive_words = "bare-directive-words",
    /// Opaque words or invocations ending a proved file-scope struct or union do
    /// not declare members. Real declaration tokens, two-word declarations, enums
    /// and uncertain conditional container ownership keep the reference's reading.
    opaque_member_tails = "opaque-member-tails",
    /// In a typedef with a word-only prefix or completed aggregate, preceding
    /// decoration words are uses; the supported final written declarator is the
    /// alias. Suffix macros expanding to attributes or nothing can reverse those
    /// roles. Pointer-after-star
    /// and unknown type-invocation prefixes retain their existing reading.
    typedef_decorations = "typedef-decorations",
    /// `(*name)(...)` in an expression records a call through the written name.
    /// A declaration prefix, pointer parameter or plain dereference keeps its reading.
    dereferenced_calls = "dereferenced-calls",
    /// C11's `_Generic` and `_Alignof` expression keywords are occurrences, not calls.
    /// Their operands and all written generic associations retain their own calls;
    /// displayed text and declaration state keep the reference reading.
    c11_expression_keywords = "c11-expression-keywords",
    /// A bounded `_Atomic(type)` specifier leaves its following declarator visible.
    /// Simple type words and pointer stars inside it are uses, not definitions;
    /// extern, typedef, function and initializer state stay with the declaration.
    atomic_types = "atomic-types",
    /// The exact preprocessing operator `defined` on an `#if` or `#elif` logical
    /// line is an occurrence, not a call. Its operands keep their existing reading;
    /// ordinary functions and macro replacement text named `defined` stay callable.
    preprocessor_defined = "preprocessor-defined",
    /// A written declaration prefix, complete parameter list and following body
    /// identify a generated-function head in macro replacement text. Its name is
    /// an occurrence, not a call; the generated definition is not resolved.
    macro_function_heads = "macro-function-heads",
    /// A simple type operand followed directly by a separate function head and body
    /// identifies a return-type wrapper use. Written return prefixes, bare void or opaque
    /// operands, nested heads and annotations keep their reading without it. The whole
    /// recognition is bounded to 256 bytes. It can hide an implicit-int GNU function with
    /// an unnamed non-void parameter followed by an empty suffix macro, so it stays
    /// outside the enabled set to preserve that definition.
    return_type_wrappers = "return-type-wrappers",
    /// A direct name followed by balanced subscript groups and a written assignment
    /// operator is an assigned occurrence. The whole suffix, including trivia and
    /// operator, must fit in 256 original source bytes. Array initializers qualify too.
    subscript_assignments = "subscript-assignments",
    /// Direct named written prefix/postfix increments and decrements, including
    /// bounded redundant parentheses. Member, indirect and subscript targets keep their
    /// reading without it, and macro and unevaluated operands are written sites.
    named_mutations = "named-mutations",
    /// An ordinary expression name in a function body may have a line end and
    /// closed block comments before `(`. The gap and initial argument check read at
    /// most 256 source bytes. Declarations, known type operands, directives, macro bodies
    /// and function heads retain their existing reading.
    split_calls = "split-calls",
    /// An `#if` right after an `#endif` opens a new group when the arm that `#endif`
    /// ended closed more braces than it opened. The reference continues the group as a
    /// pseudo-`#elif`, which starts the next arm at the group's opening depth and keeps
    /// the deepest arm's depth at its end, so the arm's closing brace is lost: after
    /// `#ifdef A } while (busy); #endif` and a following `#ifdef B ... #endif`, the rest
    /// of the file is one brace deeper, its functions are read as calls and its bodies'
    /// statements as globals. Arms that open braces, such as alternative heads written as
    /// consecutive groups, keep the reference's continuation.
    closing_arms = "closing-arms",
    /// A function head whose name is a macro taking one identifier, directly followed by
    /// the function's parameter list (`PRIV(check_escape)(const char *text) {`), also
    /// defines that identifier, which is then the function's context. The reference
    /// defines only the macro, so a search for the written name finds no definition and
    /// its calls show the macro as their caller. The macro stays defined, so nothing
    /// found before is lost. Macros with several arguments keep the reference's reading.
    written_head_names = "written-head-names",
    /// A function head written across a conditional group reads as its first arm: the
    /// look-ahead for its parameter list and body skips the `#elif` and `#else` arms of
    /// a group it opens, up to that group's `#endif`. A name followed by the group, whose
    /// first arm starts with the list (`Balloc #ifdef KR_headers (k) int k; #else (int k)
    /// #endif {`), is a definition head when its body follows. At file scope each arm
    /// starts at its group's parenthesis depth, so of alternative heads whose list goes on
    /// after the group, the last is the function. The reference allows no directive in a
    /// definition's shape, so such a function has no definition and its body reads at
    /// file scope, its locals as globals and its calls without a caller. It needs
    /// definition-bodies, which checks the body.
    conditional_lists = "conditional-lists",
    /// A function body that ends directly before an `#elif` or `#else` whose arm opens
    /// another body, in a group written between the head and its body (`f(int n) #if
    /// SLOW { ... } #else { ... } #endif`), leaves the function open, so the next body
    /// is the same function's. The reference ends the function at the first body's `}`
    /// and reads the other bodies at file scope.
    alternative_bodies = "alternative-bodies",
}

impl Corrections {
    /// The thirty-two Corrections the index is built with: every one except
    /// return-type-wrappers, which loses a genuine definition (reading-c.md §12).
    pub(crate) const ENABLED: Self = Self {
        return_type_wrappers: false,
        ..Self::ALL
    };

    /// Reads a comma-separated list of Correction names, `enabled` or `all`.
    pub(crate) fn parse(names: &str) -> Result<Self, String> {
        let mut corrections = Self::default();
        for name in names.split(',').filter(|name| !name.is_empty()) {
            if name == "all" {
                corrections = Self::ALL;
            } else if name == "enabled" {
                corrections = Self::ENABLED;
            } else if let Some(index) = Self::NAMES
                .iter()
                .position(|known| *known == name.strip_prefix('-').unwrap_or(name))
            {
                *corrections.switches()[index] = !name.starts_with('-');
            } else {
                return Err(format!("unknown correction: {name}"));
            }
        }
        Ok(corrections)
    }

    /// The names of the Corrections switched on, comma-separated in a fixed order: part
    /// of the analyzer identity an index stores.
    pub(crate) fn names(mut self) -> String {
        let on: Vec<bool> = self.switches().iter().map(|switch| **switch).collect();
        Self::NAMES
            .iter()
            .zip(on)
            .filter_map(|(name, on)| on.then_some(*name))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Every stored record of one file, in line order, read as the reference reads it (no
/// Corrections). Total: any bytes are accepted.
#[cfg(test)]
pub(crate) fn scan(bytes: &[u8], mode: Mode) -> Vec<Record> {
    scan_with(bytes, mode, Corrections::default())
}

/// Every stored record of one file with the given Corrections.
pub(crate) fn scan_with(bytes: &[u8], mode: Mode, corrections: Corrections) -> Vec<Record> {
    scan::Scan::new(bytes, mode, corrections).run()
}

/// Corpus measurements can count narrow look-aheads without modifying the source
/// (an appended canary increases the work budget). No counters enter the product.
#[cfg(test)]
pub(crate) struct ScanCost {
    pub work: usize,
    /// Heads whose body look-ahead got the narrow window past the work limit.
    pub narrow_windows: usize,
    /// Groups the closing-arms look-ahead left unread past its work limit.
    pub closing_refusals: usize,
}

#[cfg(test)]
pub(crate) fn scan_with_cost(
    bytes: &[u8],
    mode: Mode,
    corrections: Corrections,
) -> (Vec<Record>, ScanCost) {
    scan::Scan::new(bytes, mode, corrections).run_with_cost(false)
}

/// Test-only: the records without any look-ahead work limit.
#[cfg(test)]
pub(crate) fn scan_unbounded(bytes: &[u8], mode: Mode, corrections: Corrections) -> Vec<Record> {
    scan::Scan::new(bytes, mode, corrections)
        .run_with_cost(true)
        .0
}
