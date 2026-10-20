//! The remembered scan against fresh scans, the window, and the look-ahead's cost.

use super::{Bodies, Limits};
use crate::scanner::input::Input;
use crate::scanner::shapes::{LookAhead, is_word};

/// A small deterministic generator (xorshift), so failures reproduce.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "f",
    "a",
    "b",
    "int",
    "void",
    " ",
    " ",
    "\n",
    "(",
    ")",
    "(",
    ")",
    "*",
    ";",
    ";",
    ",",
    "=",
    "{",
    "}",
    "\"",
    "'",
    "/*",
    "*/",
    "//",
    "#",
    "\\\n",
    "\\\r\n",
    "x(a)",
    "x(a, b)",
    "(*p)",
    "g(void)",
    "int a;",
    "\n{",
    "struct",
    "/* a comment of some length */",
];

fn random_text(random: &mut Random) -> Vec<u8> {
    let pieces = 1 + random.below(60);
    (0..pieces)
        .flat_map(|_| PIECES[random.below(PIECES.len())].bytes())
        .collect()
}

/// The read positions right after each name, in order.
fn name_ends(text: &[u8]) -> impl Iterator<Item = usize> + '_ {
    (1..=text.len())
        .filter(|&at| is_word(text[at - 1]) && !text.get(at).is_some_and(|byte| is_word(*byte)))
}

/// Limits small enough for short texts to reach the window and the work limit.
const SMALL: Limits = Limits {
    window: 48,
    narrow: 12,
    factor: 1,
    base: 16,
};

#[test]
fn constructor_reads_charge_macro_work_and_keep_ordinary_windows() {
    let source = b"int Ctor(left, right)(void) { helper(); }";
    let mut input = Input::new(source);
    input.advance(8);
    let mut bodies = Bodies {
        limits: SMALL,
        ..Bodies::default()
    };
    assert!(bodies.macro_constructor(&input, b"Ctor", 0));
    assert_eq!(bodies.work, 0);
    assert!(bodies.macro_work >= 4 + b"(left, right)(void) ".len());
    // Repeated recognition exhausts only macro work. The narrow window cannot
    // reach the second list's body, while ordinary heads still get the full one.
    while bodies.macro_work <= source.len() + SMALL.base {
        assert!(bodies.macro_constructor(&input, b"Ctor", 0));
    }
    assert!(!bodies.macro_constructor(&input, b"Ctor", 0));
    assert_eq!(bodies.window(&input), SMALL.window);
    assert!(bodies.narrow_windows.get() > 0);
}

#[test]
fn numeric_annotations_require_complete_decimal_lists() {
    for (text, expected) in [
        (" (3.12)", true),
        ("(2 /* version */,\\\n 4)", true),
        ("()", false),
        ("(int)", false),
        ("(name)", false),
        ("(T (*cb)(int))", false),
        ("(T [10])", false),
        ("(1 + 2)", false),
        ("(\"3.12\")", false),
        ("(3.)", false),
        ("(3,)", false),
        ("(3", false),
        ("(3 /* unfinished)", false),
        ("(3\n#if FLAG\n)", false),
    ] {
        assert_eq!(
            Bodies::default().numeric_arguments(&Input::new(text.as_bytes())),
            expected,
            "{text:?}"
        );
    }
}

#[test]
fn numeric_annotations_use_the_shared_window_and_work_limit() {
    let text = b"(1, /* a comment */ 2)";
    let mut bodies = Bodies {
        limits: SMALL,
        ..Bodies::default()
    };
    assert!(bodies.numeric_arguments(&Input::new(text)));
    assert_eq!(bodies.work, text.len());
    bodies.work = SMALL.factor * text.len() + SMALL.base + 1;
    let before = bodies.work;
    assert!(!bodies.numeric_arguments(&Input::new(text)));
    assert!(bodies.work > before);
    assert_eq!(bodies.narrow_windows.get(), 1);
    assert!(!Bodies::default().numeric_arguments(&Input::new(
        format!("(1/*{}*/)", " ".repeat(64 * 1024)).as_bytes()
    )));
}

#[test]
fn remembered_scans_agree_with_fresh_ones() {
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let (mut asked, mut reused, mut narrowed) = (0, 0, 0);
    for _ in 0..30_000 {
        let text = random_text(&mut random);
        let mut bodies = Bodies {
            limits: SMALL,
            ..Bodies::default()
        };
        for at in name_ends(&text) {
            let mut input = Input::new(&text);
            input.advance(at);
            let before = (bodies.from, bodies.to);
            let parentheses = i32::from(at % 3 == 0);
            // A fresh scan with the same limits and the same work so far.
            let mut oracle = Bodies {
                limits: SMALL,
                work: bodies.work,
                ..Bodies::default()
            };
            narrowed += usize::from(bodies.work > SMALL.factor * text.len() + SMALL.base);
            let cost = bodies.cost();
            let expected_narrow = cost.narrow_windows
                + usize::from(cost.work > SMALL.factor * text.len() + SMALL.base);
            let remembered = bodies.follow(&input, parentheses);
            assert_eq!(bodies.cost().narrow_windows, expected_narrow);
            asked += 1;
            reused += usize::from(before.1 > 0 && bodies.from == before.0);
            let fresh = oracle.follow(&input, parentheses);
            assert_eq!(
                remembered,
                fresh,
                "after the name ending at {at} in {:?}",
                String::from_utf8_lossy(&text)
            );
        }
    }
    // The comparison must cover remembered answers and narrowed windows, not only fresh
    // scans.
    assert!(
        reused * 10 > asked,
        "{reused} of {asked} answers reused a scan"
    );
    assert!(narrowed > 0, "no answer used the narrow window");
}

#[test]
fn a_body_beyond_the_window_is_not_found_and_later_heads_are_unaffected() {
    let far = format!("f(void)\n/*{}*/\n{{ }}\ng(void) {{ }}\n", " ".repeat(100));
    let text = far.as_bytes();
    let mut bodies = Bodies {
        limits: SMALL,
        ..Bodies::default()
    };
    let answers: Vec<bool> = [1, far.find("g(").expect("g") + 1]
        .into_iter()
        .map(|at| {
            let mut input = Input::new(text);
            input.advance(at);
            bodies.follow(&input, 0)
        })
        .collect();
    assert_eq!(answers, [false, true]);
}

/// Asks after every name that has the reference's definition shape, as the scanner
/// would at file scope, and asks the smaller look-aheads after every name, more often
/// than the scanner would; returns the bytes scanned.
fn cost(text: &[u8]) -> usize {
    let mut look = LookAhead::new(text);
    let mut bodies = Bodies::default();
    for at in name_ends(text) {
        let mut input = Input::new(text);
        input.advance(at);
        if look.definition(&input).is_some() {
            bodies.follow(&input, 0);
        }
        bodies.direct(&input);
        bodies.linkage(&input);
        bodies.numeric_arguments(&input);
    }
    bodies.work
}

#[test]
fn the_look_ahead_costs_linear_time_on_text_built_to_be_expensive() {
    type Maker = fn(usize) -> String;
    let cases: [(&str, Maker); 11] = [
        ("numeric annotation chains", |n| {
            "A(3.12) B(2, 4) ".repeat(n)
        }),
        ("an X-macro list", |n| "FOO(a)\n".repeat(n)),
        // The head shape reads the comment to the second `*/`; the list scan to the first.
        ("lists the head shape reads differently", |n| {
            "f(a /* **/ ( */ )\n".repeat(n)
        }),
        ("heads on one line", |n| "FOO(a, b) ".repeat(n)),
        ("K&R-like declarations", |n| "f(a) int a;\n".repeat(n)),
        ("heads opening parentheses", |n| "x(a) (".repeat(n)),
        ("heads before a long comment", |n| {
            format!("{}/*{}*/", "f(a) ".repeat(n), " ".repeat(n))
        }),
        // C continues a line comment after a backslash; the scanner reads the next line as
        // code, and the look-ahead would read each such comment to the end of the file.
        ("line comments continued by a backslash", |n| {
            "f(a) // x \\\n".repeat(n)
        }),
        // Without the crlf-continuations Correction the scanner ends the directive at the
        // CR LF, which the look-ahead reads as a continuation.
        ("directives continued by backslash CR LF", |n| {
            "f(a) #x \\\r\n".repeat(n)
        }),
        ("names before a long word", |n| {
            format!("{}{}", "f(a) (".repeat(n), "w".repeat(n))
        }),
        ("names before a long literal", |n| {
            format!("{}\"{}", "extern ".repeat(n), "w".repeat(n))
        }),
    ];
    for (name, make) in cases {
        let small = make(2_000);
        let large = make(8_000);
        let (small_cost, large_cost) = (cost(small.as_bytes()), cost(large.as_bytes()));
        // Four times the text may cost at most about four times the work.
        assert!(
            large_cost <= 5 * small_cost.max(1),
            "{name}: {small_cost} bytes scanned for {} bytes, {large_cost} for {}",
            small.len(),
            large.len()
        );
        // Past the work limit each look-ahead reads within a 256-byte window per name
        // (a head's list and what follows it are two windows).
        let names = name_ends(large.as_bytes()).count();
        assert!(
            large_cost <= 16 * large.len() + (1 << 17) + 5 * 300 * names,
            "{name}: {large_cost} bytes scanned for {} bytes",
            large.len()
        );
    }
}
