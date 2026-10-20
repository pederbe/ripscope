# How Ripscope reads C

This page specifies how Ripscope's analyzer turns one source file into an Analysis unit
([ADR 0004](../adr/0004-own-scanner.md)). The baseline, with
Corrections off, targets the pinned reference cscope's observed reading. The
probe fixtures in `tests/fixtures/scanner/` record that executable's
output on small synthetic files (`examples/scanner-probes.rs`). The
[evidence index](../../evidence/claims.json) describes their scope.

Where this page and the binary disagree, the binary wins, and this page is corrected.
Probe names are cited in brackets, for example [01-basics]. A probe cited as [open: name]
is kept in `tests/fixtures/scanner/open/`: the reference's records of a reading the
baseline does not reproduce, where Ripscope defines its own behaviour (ADR 0004).

The page describes behaviour, in Ripscope's words and in Ripscope's structure. It is
written to be implemented from.

## 1. Input

- **Bytes.** The analyzer reads the file's bytes. Encodings are not interpreted.
- **Lines.** Lines end at LF. A CR is ordinary whitespace, so CRLF files read like LF
  files [15-crlf]. A missing final newline changes nothing [16-no-final-newline].
- **Line continuation.** A backslash immediately before a line end joins the two physical
  lines for the language, but each physical line still produces its own record. The
  backslash stays in the displayed text [06-macros].
- **Suffix mode.** The file name's last extension selects a mode:
  - `.l` is lex.
  - `.y` is yacc.
  - `.sd` is a state description.
  - `.bp` is a breakpoint listing.
  - Anything else is C.

  See section 10.

## 2. Tokens

- **Identifier:** a letter, `_` or `$`, then letters, digits, `_` or `$`
  [14-edge-identifiers].
- **Number:** an optional `.`, a digit, then digits, `.`, hexadecimal letters and the
  suffix letters `lLuUxX`. A `$` after a number starts an identifier, so `0x1F$` gives the
  plain symbol `$` [14-edge-identifiers].
- **Keyword:** one of the C keywords known to the reference, listed in section 8.
  Keywords are never symbols.
- **String and character literals:**
  - Their contents are opaque: nothing inside is a symbol [07-literals].
  - A backslash escapes the next character.
  - A literal left open at the end of a line ends there, and reading resumes normally on
    the next line [07-literals].
  - A backslash-newline inside a string literal continues it onto the next line. The
    reference's database then holds a raw line break inside the record, and its display
    drops part of the next line. Ripscope removes only the backslash and newline (a
    Correction) [07-literals].
- **Comments:**
  - A block comment reads as one blank. A comment spanning lines ends the current
    record at each line break, except inside a macro definition, where it does not.
  - A line comment ends the line [08-comments].
- **Whitespace:** blanks, tabs, CR, VT and FF. See section 7 for how it is displayed
  [17-control-whitespace].

## 3. Records

- **What is stored.** A record is stored for a physical line that ends up with at least
  one symbol. The symbols include empty-named markers: `}` function ends, macro ends,
  and a tag's mark at a `{` whose tag was never recorded (its name was a keyword or
  `class`, as in `struct class {`) [28-class]. A body without a tag (`struct {`) records
  no marker [13-structs].
- **Where a name or marker starts.** A name starts where the text collected so far
  ends. Two things leave that position behind: text written in place of a marker (the
  `}` of a function end, the `{` of a tag marker, a closing `%%`), and a recorded name,
  after which the position stays at the name's start until more text is collected. So
  a name written directly after a function's `}` starts at the brace (`}g` is recorded
  as a Global named `}g`), and a marker right after a name starts at the name.
- **Numbering.** The record's number is the physical line where it starts.
- **Displayed text.** The record keeps the line's text as displayed (section 7), with
  each symbol at its position.
- **Symbol order.** Symbols appear in text order, each with a role:

  | Role | Mark | Meaning |
  |---|---|---|
  | Plain | (none) | Any other identifier occurrence |
  | Define | `#` | Macro name in `#define` |
  | DefineEnd | `)` | Macro definition ends |
  | Function | `$` | Function definition |
  | FunctionEnd | `}` | Function body ends (empty name) |
  | Call | `` ` `` | Called name |
  | Include | `~` | Include operand, with its opening `"` or `<` |
  | Global | `g` | File-scope variable definition |
  | Member | `m` | Member or enumerator of a file-scope aggregate |
  | Struct | `s` | struct tag definition |
  | Union | `u` | union tag definition |
  | Enum | `e` | enum tag definition |
  | Typedef | `t` | Typedef name |
  | Class | `c` | class tag definition |

- **De-duplication on a line.** A name already recorded on the same line, with the same
  role and the same count of function definitions seen so far on that line, is not
  recorded again [12-dedup]. Empty-named symbols are never de-duplicated. Names compare
  as the line's bytes stand at that moment: once a `#define` or `#include` later on the
  line has written its code over the line's first byte, a name there no longer equals a
  later occurrence, so both are recorded [27-define-after-name].
- **Where a macro end goes.** It is normally a separate marker after the last record of
  the definition. When the definition's last recorded name was de-duplicated, it becomes
  an inline empty-named symbol at the end of the record instead [12-dedup].

## 4. Scope and state

The analyzer tracks:
- **brace depth:** it never goes below zero. A stray `}` at depth zero does nothing
  [18-unbalanced].
- **parenthesis depth:** also never below zero.
- whether it is inside a function body, a macro definition, or the body of a file-scope
  aggregate.
- the pending declaration context:
  - after `extern`, until the `;` at that depth.
  - after `=` at file scope, an initializer, until the next `,` or `;` at that depth.
  - inside `[...]`, an array dimension.
  - after `:` at file scope, a bit-field width, until `,` or `;`.
  - after `typedef`, the typedef state, until the `;` at that depth.

A `}` that brings the depth back to zero ends:
- an aggregate body, if one is open.
- otherwise the current function, recording `}` [01-basics].

Inside a class body the same applies at depth one.

The private `Declarators` module holds the form and parameter-list state inside
the cloneable declaration state. It owns bounded pointer-object and typedef
proof composition, deferred list activation and the keyword, delimiter and
statement paths that end those readings. Name preparation installs split-list
evidence before ordinary macro/head decisions. Finishing refines their reading
and handles typedef evidence in the existing order. Scanner callers consume
source, render keywords and emit records. The persistent Bodies and LookAhead
instances keep their own cache and work accounting.

List opening runs before the parenthesis counter changes. Closing runs after
it changes. Brace and statement operations keep their own termination phases.
Deferred lists activate only at a matching original source position. Macro
snapshots contain nesting plus declaration state, including Declarators.
Conditional snapshots and joins do not hold its form or parameter-list fields.

## 5. What each identifier becomes

Deciding a role often needs look-ahead beyond the identifier. The competing candidate
shapes are compared by how much text each one matches, and the longest match wins.
Among equal lengths, the order below decides. Comments count as whitespace inside these
shapes, which can reach unexpectedly far (section 9).

1. **Function definition.** The name is followed by:
   - a parenthesised parameter list holding only words, numbers, whitespace or comments
     and `* & [ ] = , . :`. The only nested parentheses allowed are a function-pointer
     parameter (the Debian rule [03-function-pointer-params, 22-nested-parameters,
     23-nested-parameters-blanks]), which has exactly this form:
     - `(`, `*`, a name and `)`, separated only by blanks, with no line ends or
       comments.
     - then blanks and `(`.
     - then any text without parentheses, line ends and comments included, up to `)`.

     Every other nested form keeps the name from being a definition: `(cb)(int)`,
     `(**cb)(int)`, `(*cb[4])(int)`, `(*)(int)`, a bare `(*cb)`, `(*cb)(int)(int)`,
     arguments that themselves contain parentheses, and a line end between the two
     parts.
   - then any run of parentheses and whitespace.
   - then a body-start character: `{`, `:`, a letter, `_` or `#`, which admits K&R
     declarations [02-kr].

   It is a Function definition when all of these hold:
   - brace depth is zero, or one inside a class.
   - the line is outside a macro definition, and it does not start with a directive
     (section 6 explains a subtlety).
   - the file is not in a lex or yacc rules section.

   Otherwise the shape is treated as a call.
2. **Call.** The name is followed by `(` and argument-like text. It is a Call inside a
   function body, a macro definition, or a lex or yacc rules section. At file scope the
   shape instead starts a *pending parenthesised form*, during which no global
   definitions are recorded, until the matching `)`, or until a keyword other than
   `typedef`, `extern`, `enum`, `struct` and `union` appears at the parenthesis depth
   where the form started. This is not yet probed, and [32-forms] shows how easily a
   file-scope `NAME(...)` followed by a word reads as a K&R function head instead.
3. **Declarator after a type word.** The name is followed by a run of `*` and whitespace,
   then another identifier. The name is recorded as Plain, and reading continues at the
   next identifier.
4. **Anything else.** At file scope the name is a Global definition, or a Member inside a
   file-scope aggregate body, unless any of these holds:
   - a macro definition is open.
   - the line is a directive.
   - `extern`, an initializer, an array dimension, a bit-field width or a pending
     parenthesised form is active.
   - a function body is open.

   In those cases it is Plain.

After the role is chosen:
- **Keywords.** A keyword records nothing. `typedef` starts the typedef state, and `extern`
  starts the extern state.
- **Tags and typedefs.**
  - An identifier that follows `enum`, `struct` or `union` and is itself followed by `{`
    becomes a tag definition.
  - A tagless `{` after one of those words marks a tagless aggregate.
  - In the typedef state, the names that are neither the type's first word nor inside an
    array dimension become Typedef names [10-extern-typedef].
- **`if (`.** `if`, then optional whitespace, then `(` is consumed without looking up
  `if` as a keyword, which affects the display (section 7) [19-if-lookahead].
- **Classes.** `class`, whitespace or comments, a name, and then only names,
  parentheses, colons and whitespace or comments up to `{`, makes the name a Class tag
  definition. Inside the class body, depth one counts as the top level for function
  definitions. `class` followed by anything else is an ordinary identifier, and a tag
  pending from `class` is not taken by a second `class` [28-class].

## 6. Preprocessor lines

`Scan` recognizes directives, consumes their source and accounts for physical
lines. Its private `ConditionalArms` module owns opening snapshots, alternate-arm
restoration and delayed closure, borrowing the live nesting and declaration
state and the existing look-ahead. It consumes no source and emits no Navigation
facts. Saved state is limited to the brace, head, call, typedef-prefix, tail and
structure readings below. Parameter-list state and macro snapshots stay outside
this lifecycle.

- **Directive mode.** A `#` followed by optional whitespace starts one, except in lex and
  yacc rules sections.
- **`#define NAME`.** Records Define. The rest of the logical line, continuation lines
  included, is a macro body read in macro context: calls are Calls, and nothing there is
  a Global. The logical line's end records DefineEnd [06-macros].
- **`#include "x"` and `#include <x>`.** Record an Include whose name keeps the opening
  delimiter, and the file is followed. Text after the operand is read normally.
  - In the reference's default database the directive's first byte is replaced by a
    keyword code, so the directive test in section 5 does not see a `#` for the rest of
    that line.
- **`#if`, `#ifdef` and `#ifndef`,** each followed by whitespace, remember the brace depth.
- **`#else` and `#elif`.** Remember the deepest brace depth reached so far in the group,
  then restore the depth from before the `#if`.
- **A lone `#`.** A `#` directly followed by the line end keeps that line end inside the
  record, so the next line joins it. The raw line end is not displayed. Like a keyword
  code, it skips the blanks and letters after it, except a keyword that already carries
  its own code [31-hash-lines].
- **The `#define` name** starts after the last space or tab of the directive, so a
  vertical tab or form feed after a space becomes part of the name. With neither, it
  starts after the blank run [31-hash-lines].
- **`#endif`.**
  - It sets the depth to the deepest of the arms.
  - **Pseudo-elif:** when the next thing after the `#endif` line, skipping only whitespace
    and comments, is another `#if`/`#ifdef`/`#ifndef`, and no `#else` has been read since
    the most recent `#if` (an `#elif` does not count), the new `#if` is treated as an
    `#elif` of the old group [04-conditionals, 05-banner-comment, 29-elif-pseudo].
- **Unspaced directives.** `#if(C)` is not recognised as a conditional, because
  whitespace is required. Its identifiers are Plain [04-conditionals].
- **Other directives.** `#undef`, `#pragma`, `#error` and the rest record their
  identifiers as Plain [06-macros].

## 7. Displayed text

- **Blanks.**
  - Leading whitespace is dropped, and so is trailing whitespace [09-display].
  - Any run of tabs, CR, VT or FF, and any run of two or more whitespace characters,
    becomes one blank [09-display, 17-control-whitespace].
  - A block comment becomes one blank.
- **Keyword spacing.** It comes from the reference's database compression, so the
  database it compares against must be the default one:
  - The keywords `#define`, `#include`, `case`, `char`, `double`, `else`, `enum`,
    `extern`, `float`, `goto`, `int`, `long`, `register`, `short`, `static`, `struct`,
    `typedef`, `union`, `unsigned` and `void` display followed by exactly one blank,
    whatever followed them. So `(unsigned)x` becomes `(unsigned )x`, and `(void)`
    becomes `(void )` [09-display].
  - `for`, `if`, `switch` and `while` display as the keyword, a blank and `(`, swallowing
    the source's own `(` and the blanks before it: `while(x)` becomes `while (x)`
    [09-display].
  - `break`, `continue`, `default`, `return` and `sizeof` display unchanged.
  - Only keyword occurrences that reached the analyzer's keyword lookup are respaced.
    `if` directly before `(` never does, so `if(x)` stays `if(x)` [09-display].
- **Control bytes.** The reference's database cannot tell a raw control byte (for
  example a form feed inside a literal, or the line end of a continued string literal)
  from a keyword code. After one it skips text the way it does after a keyword: blanks,
  then letters, then the keyword's own spacing. The continued literal in [07-literals]
  loses the next line's first word this way. The baseline reproduces the skip. Ripscope's
  own display keeps the text, through a Correction.
- **Literal bytes** are shown as they are. The reference mangles bytes above 0x7F into
  digraph pairs. Ripscope keeps them, and escapes control bytes only at the line-protocol
  output.

### How the reference stores a line

The reference stores each line as units, and some details of the display and of field
9 follow from them rather than from the text:
- **Units.** A blank, a keyword code, a pair of characters packed into one byte, or any
  other byte. A blank followed by one of ` tnerpla` is packed with it. While names are
  still to come on the line, a character from ` teisaprnl(of)=c` followed by one of
  ` tnerpla` is packed as a pair, unless the second character starts a name. Once the
  last name is written, nothing more is packed. This is why a blank run after a packed
  character can leave two blanks in the displayed text [09-display].
- **After a keyword code or a raw control byte,** the stored text continues past blanks,
  then past letters (the rest of the keyword, or for a raw byte the next word, stopping
  at another keyword's code), and, for a keyword displayed with a following blank or
  `(`, past the source's own blanks and `(`.
- **A name's assignment flag** (field 9) is decided on the units after the name, within
  its own fragment: skipping blanks (where the codes of `else`, `enum` and `extern`
  count as blanks), an assignment is a `=` not followed by `=`, a pair starting with
  `=`, one of `+ - * / % & | ^` followed by `=`, or `<<=` / `>>=` [26-assignments].

## 8. Keywords

The reference's keyword table, a compatibility fact, has:
- the compressed keywords listed in section 7.
- `do`, `auto`, `fortran`, `const`, `signed` and `volatile`, which are keywords but are
  not respaced.

## 9. Look-ahead that reaches far

Whitespace positions in the section 5 shapes, and in the pseudo-elif shape, accept
comments. A block comment shape ends at the first `*/`, except that a comment ending in
exactly `**/` is not matched by the reference's comment shape. The shape then extends to
the next `*/` anywhere later in the file. That changes which candidate is longest:
- a later banner comment can flip a pseudo-elif [05-banner-comment].
- the same effect decides definitions after a `/** doc **/` comment [08-comments].

Ripscope reproduces this without a window. A comment that can end in several places is
evaluated once per shape phase from per-file tables built from the last comment end
backwards, so each file costs linear time however its comments are arranged.

## 10. Suffix modes

- **Lex (`.l`) and yacc (`.y`):**
  - The file starts outside file scope.
  - `%{` enters file scope and `%}` leaves it.
  - The first line-initial `%%` starts the rules section and records a Function named
    `yylex` (lex) or `yyparse` (yacc), displayed as `%% /* yylex */`. In yacc it also
    records a Call to `yylex` [20-lex, 21-yacc].
  - Inside the rules section, `name(` is a Call, and preprocessor lines are not
    directives.
  - The second `%%` ends the section with a FunctionEnd and returns to file scope.
  - Files of the other modes treat these markers as a yacc file does: `%{` and `%}`
    enter and leave file scope, and a line-initial `%%` opens a `yyparse` section that
    calls `yylex` [30-sections-in-c].
- **State descriptions (`.sd`)** and **breakpoint listings (`.bp`)** are not yet probed.
  Ripscope reads them as C, except that `'` does not open a literal in `.sd` files. The
  reference's extra rules for them (`STATE` blocks, breakpoint columns) are not
  reproduced. These suffix modes are outside Ripscope's C-only product scope.

## 11. Limits

- **Long symbols.** An identifier is stored whole at any length [40-long-symbols: 249
  to 300 bytes]. The reference checks the length of what a rule matched against 250 bytes
  where the rule reaches `definition:` in `fscanner.l`, but an ordinary identifier gets
  there through `yyless(0)`, which sets that length to 0. In C only a `#define` line
  arrives with its whole match, `define`, the blanks after it and the name (in `.sd`
  files a `STATE` line too), and it passes the limit once the three together are longer
  than 250 bytes: from a 244-byte name after one blank, a 243-byte one after two (a tab
  counts as one blank, and blanks between `#` and `define` do not count). The reference then
  ends its line buffer 250 bytes after the name's start, but keeps the name's own end:
  - A name longer than 250 bytes is cut to 250, and the stored text after it is written
    from there, so the symbol takes in as many bytes of that text as the name had past
    250, up to the line's end (`#define m…x 1` with a 251-byte name: 250 bytes of the
    name and the blank). A name that starts inside the taken-in text is lost, and so are
    every later name on the line and, if the macro ends on that line, its end, since the
    reference writes the next symbol only when it reaches that symbol's start. Otherwise
    they are kept. Where a name starts exactly at the end of the taken-in text, the empty
    text between them makes the reference's queries misread the line: field 1's row stops
    after the name, and a match in field 0, 2, 3 or 9 on a name after it aborts the whole
    query with `cannot get source line from database`.
  - A name that passes the limit at 249 bytes or fewer keeps its end, and the stored line
    holds, up to 250 bytes after the name's start, the NUL that ends the copied name and
    then whatever earlier lines of the same file, or of files analysed before it, left in
    the buffer (undefined behaviour, below). A 249-byte name leaves exactly the NUL, so
    its record does not depend on what came before. The NUL reads as a keyword code: when
    nothing but NULs and blanks stand between it and a value that starts with a letter,
    the rest of the line's names and the macro's end are lost. At exactly 250 bytes the
    reference only warns, and its record is the line as written.

  The baseline reproduces neither. It stores every name whole and the line as written,
  under ADR 0004, and a unit test pins that reading against the reference's records
  [open: long-define-names]. Queries treat long names differently too. The reference
  shows function and macro names cut to 250 bytes in the function column of fields 0, 3
  and 9, matches a regular expression against only a name's first 250 bytes (an ordinary
  identifier's too), and refuses a `-L` pattern longer than 250 bytes (a session splits a
  longer request). Ripscope shows and matches names whole and takes patterns up to
  4,096 bytes.
- **Statement length.** The reference's statement-length check almost never takes effect.
  Ripscope never drops a line, and its look-ahead costs linear time (section 9).
- **NUL bytes.** The default database stores a NUL like a keyword code, so the text after
  it loses its blanks and letters, and the baseline does the same [probe: `x = 1;\0 y(z);`
  loses the call `y`]. The reference's queries then misread the database, which uses NUL
  as its block sentinel. Ripscope's queries read it safely.
- **Undefined behaviour in the reference.** Where the reference hangs (long blank runs) or
  reads out of bounds, Ripscope's behaviour is its own, and scanning stays within the
  line. After a `#define` name of 248 bytes or fewer that passes the limit (above), the
  reference's stored line holds bytes that earlier lines left in its buffer, so its record
  depends on the earlier lines of the same file and of files analysed before it. Ripscope
  stores the line as written. An include operand whose delimiter comes before earlier
  names on its line (`a < b; #include "x/../y.h"`) leaves the reference showing stale
  bytes. Ripscope drops the names after the delimiter.

## 12. Corrections

A Correction reads a construct differently from the reference because the reference's
reading is wrong ([ADR 0002](../adr/0002-measured-compatibility.md)). A clear fix is
enabled once it is measured on the corpus and loses no real definition. A Correction
that trades a correct reading for another stays off. All Corrections below except
return-type-wrappers are enabled. The index is built with the enabled ones, while the
baseline, read without them, stays the reference reading the compass compares with
cscope.
`tests/fixtures/scanner/intended/` holds the enabled reading of each probe that reads
differently from the reference's records in `expected/`, and every Correction has such
a probe. Linkage-blocks, definition-bodies, directive-continuations, crlf-continuations,
unspaced-conditionals, macro-bodies, initializer-entries, definition-heads,
macro-line-comments and extern-definitions stop "leaks": readings that set a state
the reference never undoes, or miss a function's head, so that what follows reads
wrongly. Pointer-declarators, parenthesised-typedefs and typedef-qualifiers read
declarations the reference misreads on their own line: the first two define names the
reference never defines, and typedef-qualifiers stops a false typedef. Each can be
switched on alone for measurement (`--corrections NAMES` on the hidden `--dump-units`,
`--units-query` and `--store-build` commands, or `enabled` or `all`), though several
depend on each other: alone, linkage-blocks exposes misread heads inside the block,
macro-bodies needs macro-line-comments for commented macros, macro-line-comments needs
directive-continuations, definition-heads needs definition-bodies to close the
reference's false definitions, pointer-declarators needs definition-bodies (alone it
keeps the reference's parameter Members, and also defines the pointer parameters of a
member's function-pointer parameter list), and macro-bodies needs extern-definitions
after an `extern` function definition. Alone, extern-definitions lets the reference's
other misreadings show where its `extern` state hid them: a body read at file scope,
designated initializer entries, and file-scope bodies in `extern "C"` blocks. The other
Corrections stop them.

- **linkage-blocks.** `extern`, a string literal on one line, and `{`, with only
  whitespace or comments between them, open a linkage block. Its `{` and the `}` that
  closes it leave the brace depth alone, and this `extern` starts no extern state, so the
  block's contents read as at file scope.

  ```c
  extern "C" {
  struct s { int a; };      /* reference: a plain;        corrected: a Member */
  int counter;              /* reference: plain;          corrected: a Global */
  int f(void) { g(); }      /* reference: f and g plain;  corrected: $f, `g, } */
  }
  ```
- **definition-bodies.** Where the Function definition shape (section 5) matches, the name
  is a Function definition only when a body follows its parameter list:
  - between the list and `{` may come parenthesised declarator parts, words, comments and
    directive lines, but not `struct`, `union` or `enum`.
  - after a list of plain parameter names there may instead be up to one `;`-terminated
    declaration per name, with `{` right after the last `;` (K&R). An entry may be
    several plain words, a name and a parameter macro (`len LOCALE_PARAM`), and words
    other than keywords may stand between the last `;` and `{`, a declaration macro
    (`LOCALE_PARAM_DECL`) [75-kr-parameter-macros].
  - at top level, a list that is neither typed parameters nor K&R names stops at a
    following built-in type, separated only by whitespace, closed comments or line
    splices: in `FORMAT(2, 3) void logprintf(...) { ... }`, `FORMAT` is an occurrence
    and cannot borrow `logprintf`'s body. Unknown type names and intervening
    directives keep their existing reading [60-numeric-prefixes].
  - for a head inside parentheses (as in `int (*get(void))(int)`), only what lies outside
    those parentheses ends the declaration or opens the body.
  - a head on any line of a directive is never a definition, and a head inside the
    parameter list of a head already read as a definition is a parameter's type, a plain
    name. Once a head has both a written type prefix and parameter-list evidence,
    a later annotation such as the kernel's `__acquires(lock)` cannot replace it.
    Heads without that evidence remain provisional so wrappers such as
    `FT_LOCAL_DEF(T) get(...)` keep their existing reading. Conditional arms restore
    the head state present before the group, allowing each alternative written head.

  A head without a body is a function declaration: it reads as a pending parenthesised
  form (shape 2) that lasts to a `;` or `,` at its parenthesis depth or outside it, to the
  `)` of a parenthesis around it, or to a `{`. A type or statement keyword at its depth
  ends it too, and is then also the type word a pending `typedef` waits for, so after a
  macro line without `;` (`DECLARE(x)`) the next `typedef` still defines its name. A
  keyword head without a body is only a keyword, and the same form starts after it. A
  head in a directive reads as an ordinary form. Outside functions and macro bodies, a
  `(` right after a `)` starts the same form, unless `*` or `^` follows it (in
  `MACRO(int) (*hook)(void)` that group is the declarator), so the parameter list of a
  parenthesised declarator (`(*name)(int x)`, `(name) (int x)`, a member's
  `(*cb)(struct h *h)`) declares nothing. For a proven pointer group, the list may start
  on a later line or after a closed block comment: the scanner remembers its source
  position and opens this form there
  (the limits are under pointer-declarators below). This form also
  ends at a `;` or `}` of the block it is in, at any parenthesis depth, so a parenthesis
  left open by `#if` arms cannot carry it on, and at a `{` outside its parentheses. While
  a definition's head waits for its body, a call-shaped name is not a call, except on a
  directive line (a call in an `#if` inside a parameter list is one from that
  function), in an array-bound expression, or in an explicit
  attribute's expression payload. Nested array bounds keep the same rule.

  ```c
  int f(int) __attribute__((pure));  /* reference: $f, and what follows is inside f */
  int g;                             /* reference: plain;  corrected: a Global      */
  ```
  An attribute head (`__declspec`, `__attribute__` or `__attribute`) is instead a macro
  among the declaration's specifiers, an ordinary form that ends with its list. This
  applies before declarations and function bodies alike: in
  `__declspec(dllexport) int f(void) { g(); }`, only `f` defines a function, and owns the
  call to `g` [47-attributed-bodies]. A head without a body is also a specifier when it
  begins its declaration inside a linkage block (`PyAPI_DATA(int) name;`). The name
  after it is then defined [45-specifier-macros, 46-linkage-declarations]. A word at the
  declaration's own depth begins it, but not one
  in a directive, and a `;`, `{` or `}` ends it, except the `}` of an aggregate the
  declaration declares. The reference reads such a head as a call one brace deep, and
  defines the name, only after an inner `extern` declaration: before one, the block's
  `extern` state hides its declarations. Outside linkage blocks any other head without a
  body stays a function declaration (`API_DATA(int) name;` at file scope defines
  nothing), and so does a head after another word (`char *f(int n) ATTR;`,
  `static M(T) name;`). A trailing attribute cannot be told from a declarator, so in a
  linkage block `implicit(int x) ATTR;`, `M(T) name ATTR;` and `M(x) struct tag;` define
  `ATTR`, `ATTR` and `tag` (as the reference does after an inner `extern` declaration),
  and after two macros in a row (`A(x) B(y) name;`) the name stays plain.
  An initial specifier whose complete list contains only decimal numeric operands,
  optionally fractional and comma-separated, does not begin the declaration. Thus
  `DEPRECATED(3.12) API_DATA(int) name;` defines `name` through the same specifier
  rule. Whitespace, comments and line splices may separate operands and delimiters.
  Names, types, nested lists, operators, literals, empty lists and directive boundaries
  keep their reading. This does not identify macro expansions: the existing implicit
  prototype ambiguity also applies after a numeric prefix, so
  `DEPRECATED(3.12) implicit(int) ATTR;` defines the trailing `ATTR`
  [61-annotation-chains].
  The look-ahead reads the parameter list and at most 64 KiB past it. No read, not even
  one through a comment, directive line or literal, goes past that window. It remembers
  its last scan, and a head whose parameter list that scan read at the same depth reuses
  it, so runs of heads such as X-macro lists cost linear time. Once it has read 16 times
  the file's length plus 64 KiB, each later head gets a window of 256 bytes. The
  linkage-blocks, definition-heads and numeric-argument checks read within the same
  window and count toward the same limit.
- **return-type-wrappers (disabled).** At file scope, an invocation without a written
  return-type prefix is a type specifier when its simple type operand is immediately
  followed by a separate written function head and direct body. The operand contains
  built-in type words and qualifiers, or one opaque type word with a pointer star. It
  contains no parameter name, comma or nested group. Whitespace, closed comments and line
  splices may separate the pieces, but no directive or intervening annotation may.
  The wrapper's name is an occurrence, its form closes with its own list, and the
  following head retains the type-prefix evidence. The whole check fits within
  256 bytes and uses the existing direct-head classifier with a separate work tally,
  so it cannot narrow ordinary definition recognition.

  ```c
  #define API(type) type       /* both readings: a macro definition */
  API(int) wrapped(void) { }   /* reference: $API and $wrapped; corrected: only $wrapped */
  #undef API
  int API(int value) { return value; }  /* both readings: a real $API definition */
  int call(void) { return API(3); }     /* both readings: a real API call */
  ```

  Bare `void` or opaque operands (`API(void)`, `API(T)`), existing written return prefixes, prototypes,
  suffix annotations and nested wrapped heads retain their established reading.
  In particular, a list ending in a named typed inner list is unresolved, preserving
  the inner-head reading kept under [ADR 0002](../adr/0002-measured-compatibility.md).
  This rule performs no macro expansion and changes no displayed text
  [70-return-type-wrappers].

  A void-only operand can be a real C89 function's parameter list followed by an
  empty suffix macro: `API(void) SUFFIX(void) { ... }` with `#define SUFFIX(arg)`.
  Preserve `API`'s definition in that shape. Suppressing it as a wrapper would lose
  a correct definition. A GNU implicit-int definition with an unnamed non-void
  parameter can still be lost: `API(int) SUFFIX(void) { ... }` with the same empty
  suffix macro has exactly the wrapper shape. The paired fixture in
  `tests/fixtures/scanner/proposed/` shows that loss. The Correction stays off to preserve that definition (ADR 0002). It
  remains available for measurements.

  Ripscope does not read included headers to classify macros, because every unit would
  then depend on the headers it might include ([ADR 0003, "Each file is read as written,
  without a compiler or preprocessor"](../adr/0003-read-source-as-written.md)).
- **subscript-assignments.** A direct written name followed by balanced subscript
  groups and an assignment operator is assigned, including pointer-indexed storage.
  Written array initializers qualify too. An empty dimension needs a definition mark
  or a simple written declaration prefix. All eleven assignment operators qualify,
  while comparisons and increment/decrement do not. Nested scalar and subscript
  writes retain their independent flags, and calls and reads inside brackets remain.
  The suffix, including trivia and operator, fits within 256 original source bytes.
  A separately bounded preceding span checks declaration prefixes and indirect/member
  bases. Closed comments, quoted tokens and LF/CRLF splices are handled without joining
  punctuation across ordinary whitespace or comments. Unfinished delimiters, a brace,
  statement separator, directive or the byte bound stops recognition. Member,
  parenthesized, indirect and call-result bases and terminal members are excluded.
  The flag is recorded before same-line deduplication. An earlier read cannot hide
  the later store or supply its context [71-subscript-assignments].
- **named-mutations.** Direct named prefix/postfix `++` and `--` contribute assigned
  occurrences, with bounded redundant parentheses and trivia around their operands.
  Named pointer mutations qualify, including `*pointer++`. Member, indirect and
  subscript mutation targets do not. Cast results are not direct named operands.
  Mutation recognition uses at most 256 original source bytes on each side of the
  name and retains the actual occurrence before deduplication, independently of
  definition look-ahead. Longest sign tokens distinguish `other+++name` from a
  prefix mutation of `name`. Incomplete trivia and uncertain window boundaries
  stop recognition [72-named-mutations].
  Written macro and unevaluated operands such as `sizeof(counter++)` qualify. This and
  subscript assignment follow [ADR 0012, "Assignment search includes subscript stores,
  increments and decrements"](../adr/0012-written-assignment-search.md).
  Both recognizers account for comment delimiters formed by LF/CRLF splices. An
  opener visible in the preceding window without a close before the name cannot
  establish an assigned occurrence, even if the scanner retained that name.
  A shared forward cursor remembers the completed lexical span at the window's
  left edge. When that edge clips an earlier closed comment or literal, only its
  remaining bytes are masked for assignment recognition. Target syntax keeps its
  original byte positions and budget. An unknown preceding operand boundary still
  stops recognition. The cursor retains constant state and advances monotonically
  across successive windows. It does not change Displayed text or other marks.
- **split-calls.** An ordinary expression name inside an established function body
  can precede `(` across line ends and closed block comments. The gap, opening
  parenthesis and initial argument check fit within 256 original source bytes.
  A known type or qualifier cannot start a call argument, so an annotation followed
  by a cast such as `(void) expression` keeps the reading without this Correction.
  Declaration prefixes keep local prototypes and pointer declarations out. Initializers
  and array bounds can contain calls. Function heads, macro bodies and directives keep their existing
  reading, as do line comments and splices in the gap. The rule changes only the
  called name's mark, retaining its physical line, Context and Displayed text
  [74-split-calls].
- **written-head-names.** A function head whose name is a macro taking one identifier,
  directly followed on the same line by the function's own parameter list
  (`PRIV(check_escape)(const char *text, int length) {`), also marks that identifier as
  a Function, so it is defined and is the context of the body's names. The macro stays a
  Function too. A keyword argument, several arguments and a head without the second
  list keep the reference's reading ([ADR 0003](../adr/0003-read-source-as-written.md))
  [78-written-head-names].
- **conditional-lists.** The look-ahead that reads a head's parameter list and what
  follows it up to the body, for definition-bodies and definition-heads, reads only the
  first arm of a conditional group it opens itself: at such a group's `#elif` or `#else`
  line it skips to the line after the group's `#endif`, counting groups nested in the
  skipped arms, within the same window. A group open before the head keeps all its
  arms, so a macro line ending one arm (`IMPL (bzero, 1) #else ... #endif`) does not
  borrow a later function's body. A scan that skipped an arm is not reused by a later
  head. A name followed, with only whitespace or comments between, by an `#if`,
  `#ifdef` or `#ifndef` line whose arm starts with `(` is a definition head when its
  body follows, as with definition-heads. So a list or K&R declarations written once per
  arm read as one head: `Balloc #ifdef KR_headers (k) int k; #else (int k) #endif {`,
  `connect(int port, #if V5 int flags, int extra) #else int flags) #endif {`, and
  `scale(n) #ifdef WIDE long long n; #else long n; #endif {`. The reference allows no
  directive in a definition's shape, and definition-bodies counts both arms'
  declarations, so the function has no definition and its body reads at file scope.
  At file scope, outside a head, each arm also starts at its group's parenthesis
  depth, so of alternative heads whose list goes on after the group (`#ifdef R
  scan2(const char *s, #else scan(const char *s, #endif int *width) {`) the last is the
  function and the context of its body, as with alternative heads whose lists close in
  their arms. The reference reads every later head inside the first one's list
  [79-conditional-lists].
- **alternative-bodies.** A function body's closing `}` leaves the function open when an
  `#elif` or `#else` line follows it, and then `{`, with only whitespace or comments
  around the line, in a group opened between the function's head and its body: `void
  residual(int n) #if SLOW { ... } #else { ... } #endif`. The next arm's body is then the
  same function's, and its own `}` ends it. The reference ends the function at the first
  body and reads the others at file scope, their locals as globals and their calls
  without a caller [80-alternative-bodies].
- **closing-arms.** The pseudo-`#elif` (section 6) does not continue a group when the
  arm its `#endif` ends closed more braces than it opened, or when the following arm
  would close a brace before opening one: the two groups are read one after the other.
  The following arm is read up to its own `#elif`, `#else` or `#endif`, counting braces
  outside comments and literals and, in a group nested in it, only in the first arm,
  within 64 KiB and a look-ahead total of 16 times the file's length. The group that
  opens after such a split ends with its first arm's depth instead of its deepest arm's,
  as with every first arm taken. The reference loses the closing brace, so the rest of
  the file reads one brace deeper (`#ifdef ASYNC } while (busy); #endif` before
  `#ifdef BLINDING`), or a later `}` in the following group ends the function early.
  Alternative heads written as consecutive groups, whose arms open braces, keep the
  reference's reading [76-closing-arms].
- **attribute-expressions.** GNU `__attribute__` / `__attribute` and Microsoft
  `__declspec` introducers and their outer labels are ordinary occurrences, never
  calls or definitions. Calls nested in a label's expression keep their written
  function or macro context. An attribute before a function head is at file scope.
  Attributes between an aggregate keyword and its tag or body do not hide the
  aggregate, so its fields remain Members. This look uses the function-head work
  budget and window. Keyword-spelled labels retain their usual displayed spacing.
  Closed block comments may separate an introducer from its opening parenthesis
  within the 256-byte declarator window. Spliced, unterminated and longer comment
  gaps keep the reading without this Correction. The existing whitespace-only boundary
  is preserved [52-attribute-expressions, 63-attribute-comment-gaps].
- **conditional-comments.** A closed multiline block comment starting after
  `#else` or `#endif` cannot produce symbols, calls, assignments or include operands.
  The tail must consist only of whitespace and closed block comments, ending within
  64 KiB. Code on the closing line, an unterminated comment, a longer tail or a
  backslash-newline splice inside a comment keeps the reference reading. Excluding
  splices avoids mistaking a split `*/` for a later comment's end.
  [53-conditional-comments]
- **keyword-macros.** A macro's name is recorded before C keyword lookup, so
  `#define const` and other keyword-spelled macro names are Definitions. Their
  displayed text retains the reference's keyword spacing. [54-keyword-macros]
- **directive-continuations.** A directive's continuation lines read as its first line
  does, and so does the rest of its own line where the reference reads code: after an
  `#include` operand (`#include "a.h" int x;`, whose trailing tokens compilers ignore) or
  a `#` in mid-line. Nothing on them is a definition. A `#` right before a line end ends
  the directive there, though the next line joins its record.

  ```c
  #if defined(A) || \
      (X < 5)                /* reference: X is a Global; corrected: plain */
  #endif
  ```
- **crlf-continuations.** A backslash before CR LF continues the line, in code and in
  literals, as a backslash before LF does. The reference takes the CR for a blank after
  the backslash, so in a CRLF file a multi-line macro ends on its first line and the rest
  reads as code at file scope.
- **unspaced-conditionals.** `#if` and `#elif` directly followed by punctuation (`#if(A)`,
  `#elif!defined(B)`) are conditionals, but `#ifdef(` and `#ifndef(` are not. Unlike a spaced
  `#if` right after an `#endif`, which the reference continues as a pseudo-elif
  (section 6), an unspaced one opens a new group.
- **macro-bodies.** A macro definition is read on its own, from a fresh brace and
  declaration state, and the state from before it returns where it ends. The reference
  carries a macro's braces and any `typedef`, `extern` or initializer it starts into the
  rest of the file:

  ```c
  #define CHECK(c) typedef int check[(c) ? 1 : -1]
  #define N 1                /* reference: N is a typedef; corrected: a macro */
  int g;                     /* reference: a typedef;      corrected: a Global */
  ```
  An unfinished character or string token cannot extend the replacement list
  across an unspliced newline. The macro ends there and the enclosing function,
  aggregate or declaration state returns [62-macro-quoted-newlines]. Closed
  literals and backslash continuations keep their existing reading.
- **initializer-entries.** An `=` inside an initializer's braces leaves the initializer
  open, so designated entries define nothing.

  ```c
  const struct ops o = {
      .a = x,
      .b = y,                /* reference: b is a Global; corrected: plain */
  };
  ```
- **definition-heads.** A name at file scope, outside parentheses, directives and
  keywords, followed by a balanced parenthesised list and directly by `{` (only
  whitespace, comments and directive lines between) is a Function definition, also where
  the Function definition shape rejects the list, unless a definition is already open.
  The reference rejects lists with nested parentheses or a line break inside a
  function-pointer parameter, and then reads the body at file scope: its locals become
  Global definitions and its calls are lost. A second list may stand between the first and
  `{`, as after a macro's argument list (`DEFUN ("name", Fname, Sname, 1, 1, 0, doc:
  /* ... */) (Lisp_Object args) {`). The name is then the function, as for a one-list
  macro head [77-two-list-heads]. Not promoted: an attribute
  (`__attribute__`, `__declspec`), a name right after `struct`, `union` or `enum`, and a
  wrapper whose list holds a whole function head (`UV_UNUSED(static int f(int a))`,
  `__NTH (tolower (int c))`: no comma of its own, and ending in a name with a typed
  list), which is left to the reference's reading of the inner head (the definition,
  where the inner head's own shape allows).

  A direct callback parameter combined with a suffix annotation remains a
  recognition limit: `int f(int callback(int)) ATTR { ... }` can give the body to
  `callback`. Promoting `f` through the suffix could instead hide a real function
  in the identical outer shape:

  ```c
  #define WRAP(x) WRAP_##x
  #define WRAP_int
  #define ATTR
  int WRAP(int wrapped(int x)) ATTR { return x; }
  ```

  Token pasting makes this a definition of `wrapped`. The inner-head reading is kept
  under [ADR 0002, "Compatibility with cscope is measured, and its misreadings are
  corrected"](../adr/0002-measured-compatibility.md).

  ```c
  int count(STACK_OF(X509) *sk)  /* reference: plain;    corrected: $count     */
  {
      int n = 0;                 /* reference: a Global; corrected: a local    */
      return total(sk, n);       /* reference: plain;    corrected: `total     */
  }                              /* corrected: the function's end              */
  ```
  In the reference's reading a declaration it takes for a definition stays open until
  the next body ends, so alone this Correction promotes nothing there. With
  definition-bodies, such a declaration closes at its `;`. With definition-bodies, a head
  inside a promoted head's list is a parameter's type: `Py_UNUSED` in
  `f(PyObject *o, void *Py_UNUSED(x)) {`, which the reference takes for the definition
  owning the body.
- **macro-line-comments.** A line comment on a macro definition's line ends with the
  line, and so does the macro, unless a backslash ends the comment (C continues it onto
  the next line). The reference takes the line end into the comment, so the macro runs on
  through the next line: that line defines nothing, and its names and calls take the
  macro's context. The blanks before such a comment are trailing blanks, so the displayed
  line is the reference's display of the line without the comment.

  ```c
  #define LIMIT 16 // entries
  static int table[LIMIT];   /* reference: inside LIMIT;  corrected: a Global */
  ```
- **pointer-declarators.** The name of a pointer declarator, to a function or to an array,
  is a Member in an aggregate and a Global at file scope, wherever a bare name there
  would be one. Its basic one-line reading is `(`, one or more `*` or `^` with
  blanks or qualifiers (`const`, `volatile`, `restrict` and their `__` spellings) among
  them, the name, any array dimensions, `)`, blanks, then the `(` of a parameter list or
  the `[` of an array (`T (*rows)[4];`), each part within 256 bytes of the name, so a line
  of many costs linear time. Its parentheses must be the ones right after a type word
  (`T (`, `int (`) that opens the declaration's form at the statement's own depth, so a
  declarator inside a parameter list, even one on the next line, declares nothing.
  Neither does one in an `extern` declaration, an initializer, after a `:`, or in a
  function body, directive or macro. The reference leaves the name plain wherever the word
  right before the group opens a form: in an aggregate it reads a typedef name before `(`
  as a call and defines a function pointer's parameters as Members instead, and at file
  scope it reads `T (` as the head of a function `T` (as a call before `()`, a `^` group or an
  array's `[`), and `int (` before a parameter list as a keyword head
  [41-pointer-declarators]. Otherwise it defines the name, and a function pointer's
  parameters too: after a keyword type in an aggregate (`int (*compare)(int);`), after a
  keyword type for a pointer to an array at file scope (`int (*grid)[4];`), after a `*`
  (`char *(*f)(void);`) or after a closed invocation (`MACRO(int) (*f)(void);`). The
  parameters are what definition-bodies stops.

  Decoration words before the first explicit `*` or `^` also count, as in
  `T (WINAPI *fp)(int);` or `HRESULT (STDMETHODCALLTYPE __RPC_FAR *query)(T *, U);`
  [49-decorated-pointers]. This extension requires aggregate or actual file scope,
  including when the reference misses a function head and reads its body at file scope.
  The suffix must start on the name's line and show a parameter list within the same
  256-byte forward window: an empty list, a keyword type (optionally qualified) at its
  start, or a comma outside nested parentheses and brackets. The comma search stops at
  comments, literals and declaration boundaries. This keeps type expressions such as
  `TYPEOF(left *right) (alias)` from defining `right`. Unknown one-parameter lists,
  words after or among the pointer operators, and a word replacing the operator
  (`APIENTRYP`) keep the reading without this extension in this object-declarator rule.
  The typedef-parameters rule below handles proved typedef groups separately.
  A nested declarator (`(*(*fp)(int))(double)`) retains the reference's reading.

  With definition-bodies, a pointer group's parameter list may instead start on a
  later line or after closed block comments [50-split-pointers, 57-pointer-trivia].
  Comment gaps stay within the same window and cannot contain line splices. The group must be the
  outermost parentheses, at the statement's own depth and outside any enclosing
  declaration form, in actual file or aggregate scope. Function typedefs keep their
  existing reading. Members inside a typedef's aggregate may qualify. The same
  structural proof must identify the suffix as a parameter list, and the entire list
  must close within 256 bytes after the name, without braces, comments, literals,
  directives or semicolons. Inline aggregate bodies are not read this way, so their
  real members are not lost. Incomplete or ambiguous lists and array suffixes
  on a later line stay unchanged. With a builtin type or explicit return pointer
  before the group, a single parameter beginning with a type word followed by a
  pointer or name (`node *p`, `size_t bytes`) also proves the list. That extra evidence
  is insufficient after an arbitrary type-expression call such as `TYPEOF(*ptr)`.
  The declared name becomes a Member or Global, and definition-bodies makes the
  list's names and suffix annotations plain occurrences. Its form lasts to the comma
  or semicolon, so the next declarator still declares normally.

  ```c
  struct ops {
      uint64_t (*hash)(const void *key);  /* reference: Member key; corrected: Member hash */
  };
  size_t (*hook)(void);      /* reference: Function size_t;  corrected: Global hook */
  ```
- **parenthesised-typedefs.** In a typedef whose type is a keyword right before a
  parenthesised declarator (`typedef void (*f)();`, `typedef int (name)();`,
  `typedef int (*A)[4];`), the keyword is the type and the declarator's name is the
  Typedef. The reference reads such a keyword as a call head wherever its definition shape
  rejects what follows the group (`()`, a comment, `[4]`, `;`), and a keyword that ends
  such a form does not count as the typedef's type, so the next name takes the type's
  place and becomes a Global [42-parenthesised-typedefs]. The keyword counts when the
  group holds nothing before the name but `(`, `*`, `^`, qualifiers and blanks, and closes
  after it on its line (array dimensions allowed), each part within 256 bytes. A word
  before the name (`( __interrupt __far * pxISR )`, `(RENAME(name))`), a name on a later
  line and a nested declarator keep the reference's reading. Field 1 shows both
  Globals and Typedefs as definitions, so changing between those marks alone
  does not change its result row.
- **typedef-parameters.** A recognised function typedef's parameter names and type
  names are ordinary occurrences, not new Typedefs. The declared name remains a
  Typedef, and a tag defined inside its list keeps its own mark [48-typedef-parameters].
  A direct list must begin with a builtin type keyword or `struct`, `union` or `enum`,
  optionally after qualifiers, within 256 bytes of the name. A leading comment or
  unknown type word retains the existing reading. A pointer or plain parenthesised
  group immediately after a builtin type keyword also qualifies
  (`typedef void (*handler)(int code);`, `typedef int (alias)(node *p);`). Both
  forms cover nested callback parameters once the list is recognised. The pointer
  group uses the same qualifiers and 256-byte windows as pointer-declarators. Its closing
  parenthesis and the opening of the parameter list must be on the name's line. A truncated word at
  the window's edge never counts as a type keyword. After the list opens, its matching
  close ends the parameter reading, so later declarators and declarations retain
  their names. Directives and macro bodies keep their existing reading.

  A plain parenthesised alias or a group decorated before its explicit pointer
  operator also qualifies when a complete suffix is visibly a parameter list: empty,
  starting with a keyword type, or containing an outer comma within 256 bytes
  [56-typedef-forms]. The return type can be a typedef name or tag, and groups after a
  comma use the same proof. Decoration words before the star are occurrences. A tag
  before a pointer-return function typedef is a type use only when the following
  declarator and parameter suffix prove it. A conditional aggregate definition keeps
  its tag. Comments, inline aggregate bodies, nested function-producing declarators
  and opaque single-parameter suffixes keep the reading without this extension.

  `typedef TYPEOF(*ptr) (alias);` can declare `alias`, and `typedef int API (*x)(void);`
  can declare `x`, so neither group is assumed to be a parameter list. These limits
  preserve real names while correcting the unambiguous forms. The same protection
  applies to decorated aliases such as `TYPEOF(*ptr) (API alias)` and
  `TYPEOF(*ptr) (API *alias)`. Two words or a pointer inside that group do not prove
  a parameter list after an arbitrary type-expression call.

  A builtin return type or explicit return pointer before `(decoration alias)`
  also proves the alias when a complete parameter suffix follows [58-declaration-tails].
  Exactly one opaque decoration is permitted, whatever its spelling. A named
  type followed by a parameter name or pointer qualifies here as well. All parts
  fit the 256-byte declarator window on one line. This covers
  `typedef void (APIENTRYP callback)(Number x)` without macro expansion.

  ```c
  typedef int callback(int count);       /* reference: Typedef count; corrected: plain */
  typedef void (*handler)(const T *arg);  /* reference: Typedef T and arg; corrected: plain */
  ```
- **typedef-qualifiers.** A qualifier (`const`, `volatile`) right after `typedef` is not
  its type, so the type name after it is a plain name, not a Typedef: `foo_t` in
  `typedef const foo_t bar_t;`, and the tag in `typedef const struct tag *tag_ptr;`. The
  reference takes any keyword for the type and defines every later name as a typedef
  [43-typedef-qualifiers]. The name after the qualifier is the declarator only where `;`,
  `,`, `[` or `)` follows it on its line (`typedef const x_t;`, an implicit `int`), and a
  body after the qualifier is the type (`typedef const struct tag {} name;`). It
  preserves the intended reading of `typedef const sds (alias);`: only `alias` is
  a Typedef. The paired [43-typedef-qualifiers] fixture records the reference and
  intended readings.

  ```c
  typedef const png_struct *png_const_structp;   /* reference: png_struct a Typedef; corrected: plain */
  typedef void (*init_plugin)();                 /* reference: a Global; corrected: a Typedef */
  ```
- **typedef-decorations.** With a word-only type prefix or a completed aggregate,
  earlier decoration words are occurrences when a later complete written alias is
  visible within the 256-byte declarator window: `typedef int API alias;`,
  `typedef T A B alias;`, `typedef int API *alias;` and
  `typedef int API (*alias)(void);`. Supported arrays and scalar declarator lists,
  local typedefs and closed comment gaps follow the same preference. Parameter and
  subscript names keep their own rules. Conditional arms restore their prefix state.
  Their join retains an opaque prefix or pointer seen in any arm.
  A pointer already before the candidate (`typedef int *API alias;`), an unknown
  type invocation (`typedef RET_TYPE(int) actual ATTR;`) and incomplete or unsupported
  groups keep the reading without this Correction. Long or spliced inter-token gaps keep
  it too. Complete array and parameter suffixes use the existing bounded delimiter reading,
  which accepts physical splices inside them. The parenthesized-name
  exceptions above stay unchanged. Known GNU/MS suffix attributes preserve the
  preceding alias.
  This prefers the written alias, without expanding macros. It can
  lose the real alias when an unknown suffix macro expands to an attribute or nothing:
  `typedef int actual ATTR;` chooses `ATTR`, even when preprocessing would leave
  `actual`. Probe 64 and stored-query tests pin that ambiguity alongside the corrected
  prefix readings [64-typedef-decorations].
- **extern-definitions.** An `extern` still pending when a function body closes at file
  scope ends there: the one before an `extern` function definition, or an unterminated
  one before the function, since no body can sit inside an `extern` declaration. The
  reference keeps its `extern` state until a `;` at the depth of the `extern`, and a
  body's own `;` are deeper, so the next declaration after `extern int f(int a) { ... }`,
  up to the next file-scope `;`, defines nothing [44-extern-definitions]. With
  macro-bodies, a `;` in a later macro no longer ends that state either, so without this
  Correction the loss would run on (the members of Ruby's `st_str_end_key` in
  `regparse.c`).

  ```c
  extern int second(void) { return 0; }
  int after;                 /* reference: plain;  corrected: a Global */
  ```
- **dereferenced-calls.** An explicitly dereferenced written name followed by an
  argument list, `(*callback)(...)`, is a Call in an expression. Whitespace and
  comments can split the form across physical lines. The name's own line holds
  the Call. The closing name group and opening argument list must be visible
  within the 256-byte window. Known type casts and control conditions allow the
  following expression. Directives preserve the surrounding context, and
  conditional alternatives restore and conservatively join declaration evidence.
  Written declaration prefixes protect their complete comma lists, including
  qualified, array and nested pointer declarators, while initializers and array
  bounds remain expressions. Plain dereferences and pointer declarations gain no
  calls. This records the written name, without resolving its runtime target.
  Member expressions, arbitrary typedef casts and more deeply wrapped callees
  keep the reading without this Correction [65-dereferenced-calls].
- **c11-expression-keywords.** `_Generic` and `_Alignof` are expression keywords,
  not callable names. Where the reference records either exact spelling as a Call,
  this Correction keeps it as a Plain occurrence. It changes no declaration state
  or displayed text and consumes no operand: nested selections, every written
  generic association and calls in alignment type operands retain their calls.
  Both conditional arms and macro replacement text remain written-source navigation
  under ADR 0003, "Each file is read as written, without a compiler or preprocessor".
  The attribute-expression path follows the same rule. Similar identifier spellings
  remain callable. Disabling the Correction restores the reference marks
  [66-c11-expression-keywords].
- **atomic-types.** `_Atomic(type)` is a type specifier rather than a function
  head. A complete operand of type words and pointer stars, including whitespace
  and closed block comments, must fit within 256 bytes from the keyword's start.
  The keyword and operand names are Plain occurrences, with normal displayed
  text. Keywords in the operand do not change declaration state. The surrounding
  declaration still controls globals, members, typedefs, externs, prototypes and
  locals, and the declared name keeps its physical line. The qualifier spelling
  `_Atomic int` stays unchanged. Macro replacement text, nested declarators,
  directives, line splices in comments, incomplete operands and operands beyond
  the window keep the reading without this Correction. This is bounded syntax recognition,
  without type resolution or macro expansion, following
  [WG14 N1570 section 6.7.2.4](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf).
  Disabling the Correction restores the reference marks [67-atomic-types].
- **preprocessor-defined.** The exact preprocessing operator `defined` in a
  line-initial `#if` or `#elif` expression is a Plain occurrence wherever the
  reference records it as a Call. A small logical-line flag follows the existing
  directive reader, including backslash continuations and unspaced conditionals.
  It ends with the directive and adds no look-ahead. Operands are read normally,
  so nested written calls and every conditional branch retain their contexts and
  physical lines. Ordinary functions named `defined`, macro replacement text,
  other directives, mid-line hashes and similar spellings keep their reading.
  Displayed text and declaration state are unchanged. This recognises the written
  operator without preprocessing or macro expansion. Disabling the Correction
  restores the reference marks [68-preprocessor-defined].
- **macro-function-heads.** Inside macro replacement text, a written declaration
  prefix on the same physical line, a complete balanced parameter list and an
  immediately following `{` identify a generated-function head. Its name is a
  Plain occurrence rather than a Call. The existing direct-head classifier reads
  at most 64 KiB, falling back to 256 bytes after its work limit. Macro recognition
  has its own work tally, so it cannot narrow later ordinary definition checks.
  Closed comments and LF/CR LF splices may separate the parts, but an unspliced
  newline, including one inside a comment, ends the look. Literal and directive
  payloads are not parameter lists. Calls in the body retain their macro context,
  physical lines and displayed text. Actual invocation parameters, generated
  prototypes, suffix annotations and prefixes on another physical line keep
  the reading without this Correction. The generated definition's identity stays
  unresolved, and no macro expansion or name list is used. Disabling the Correction restores the
  reference marks [69-macro-function-heads].
  A token-constructed head also qualifies when its initial replacement-text
  prefix fits in 256 original source bytes and contains only macro trivia,
  optional `static`, `extern`, `inline`, `const`, `volatile`, `restrict`, and
  either explicit builtin type words or one opaque nonkeyword type identifier.
  Those type alternatives cannot mix. Pointers, tags, attributes and other
  intervening tokens invalidate the whole prefix. Recognition never restarts
  later in the body. A nonkeyword constructor must have exactly two bare
  nonkeyword identifier operands and then one separate parameter list accepted
  by the same direct-head classifier, immediately followed by the body.
  Prefix trivia may cross LF/CR LF splices, including a written `Datum` before
  `CppConcat(...)(PG_FUNCTION_ARGS)`. Prefix and list scans charge the isolated
  macro work tally. Prototypes, suffixes, unfinished or additional lists,
  expressions, out-of-bound inputs and genuine body invocations keep their
  existing reading. The direct-name rule remains unchanged, including its
  split-prefix limit. Adjacent splices between a macro's name and formal list
  preserve that list's boundary. Whitespace or a comment there starts object
  replacement text [73-macro-constructor-heads].
- **bare-directive-words.** A single otherwise opaque word at file scope, followed
  only by whitespace and a directive within 256 bytes, is an occurrence rather
  than a Global. Declaration evidence continues across lines and is restored for
  conditional alternatives. Incompatible joins disable suppression. Two or more
  words can continue a declaration through a directive and retain their marks.
  For example, the two words below are a type and declarator when `A` is defined,
  `BEGIN` expands to `int`, and `T` expands to a comma:

  ```c
  #ifdef A
  BEGIN
  END
  #endif
  T x;
  ```

  Both `END` and `x` are real globals in that expansion. This reading is kept under
  [ADR 0002, "Compatibility with cscope is measured, and its misreadings are
  corrected"](../adr/0002-measured-compatibility.md).
- **opaque-member-tails.** At the closing brace of a positively recognised
  file-scope struct/union, suppress a would-be Member for one word, at least three
  bare words, or a numeric invocation followed by one word. Known C declaration
  words and reserved implementation words block this reading, as do words inside
  an invocation, pointer/array/bit-field punctuation, literals and splices.
  A two-word type/declarator pair is preserved even across a line break. Only the
  recognised container's exact depth qualifies. Nested, enum and uncertain
  conditional bodies do not. Conditional alternatives carry and conservatively
  merge the same declaration evidence. The trailing whitespace look is bounded
  to 256 bytes and no source text changes [59-opaque-boundaries]. The rule has a
  deliberate limit: macro-expanded types with no final
  semicolon, such as `TYPE_FROM(0) member` and `QUALIFIER T member`, can still lose
  a Member mark. Both readings are pinned alongside preserved explicit types.

The definition-heads rule can replace a provisional macro head only when the
current line supplies a return prefix and the new head has typed parameters.
This recovers the written function after X-macro heads without lending an earlier
type keyword to a trailing annotation [58-declaration-tails]. The pointer-declarators
rule also makes a return-type word plain when an explicit pointer, pointer group
and complete parameter suffix prove its role. A single named-type parameter is
sufficient. The type word itself is never the declared object.
