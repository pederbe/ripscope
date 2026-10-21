//! The scanner against the reference's probe fixtures (tests/fixtures/scanner/).

use super::{Corrections, Mode, Record, reference_view, scan, scan_with};
use serde_json::Value;
use std::fmt::Write as _;
use std::path::Path;

#[test]
fn ordinary_split_calls_keep_the_identifier_line_and_real_definitions() {
    for newline in ["\n", "\r\n"] {
        let source = include_str!("../../tests/fixtures/scanner/probes/74-split-calls.c")
            .replace('\n', newline);
        let records = scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED);
        let marked = |mark| {
            records
                .iter()
                .flat_map(move |record| {
                    record
                        .symbols
                        .iter()
                        .filter(move |symbol| symbol.mark == mark)
                        .map(move |symbol| {
                            (
                                record.line,
                                record.text[symbol.at..symbol.at + symbol.len].to_vec(),
                            )
                        })
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            marked(crate::unit::Mark::Call),
            vec![
                (2, b"leaf".to_vec()),
                (3, b"leaf".to_vec()),
                (5, b"leaf".to_vec()),
                (10, b"leaf".to_vec()),
            ]
        );
        assert_eq!(
            marked(crate::unit::Mark::Function),
            vec![
                (1, b"leaf".to_vec()),
                (2, b"ordinary".to_vec()),
                (3, b"grouped".to_vec()),
                (4, b"split".to_vec()),
                (8, b"assigned".to_vec()),
            ]
        );
    }
}

#[test]
fn split_calls_preserve_declarations_macros_directives_and_definition_reading() {
    let without = Corrections {
        split_calls: false,
        ..Corrections::ENABLED
    };
    let candidate = Corrections::parse("enabled,split-calls").unwrap();
    for newline in ["\n", "\r\n"] {
        for source in [
            "int prototype\n(int);\nint definition\n(int x) { return x; }\n",
            "void f(void) { int prototype\n(int); T opaque\n(int); }\n",
            "void f(void) { int (*pointer\n(int))(void); }\n",
            "void f(int callback\n(int), int value) { callback(value); }\n",
            "#define HEAD(NAME) int NAME\n(void) { body(); }\n",
            "#define M() leaf\\\n(1)\nvoid f(void) {\n#define LOCAL() leaf\\\n(1)\n leaf(1);\n}\n",
            "void f(void) {\n#if FLAG\n leaf(1);\n#endif\n}\n",
            "void f(void) {\n#if leaf\n(1)\n#endif\n}\n",
            "void f(void) { leaf\\\n(1); leaf /* comment */ (2); }\n",
            "void f(void) { leaf /\\\n* comment */ (1); }\n",
            "void f(void) { leaf /* comment \\\n */ (1); }\n",
            "void f(void) { leaf\n#if FLAG\n(1);\n#endif\n}\n",
            "void f(void) { leaf\n+ (1); leaf\n[1]; }\n",
            "void f(void) { MARKER\n(void) leaf(1); }\n",
            "void f(void) { leaf /* unfinished\n(1); }\n",
        ] {
            let source = source.replace('\n', newline);
            assert_eq!(
                scan_with(source.as_bytes(), Mode::C, candidate),
                scan_with(source.as_bytes(), Mode::C, without),
                "{source}"
            );
        }
    }
}

#[test]
fn split_calls_preserve_builtin_and_qualified_cast_operands() {
    let without = Corrections {
        split_calls: false,
        ..Corrections::ENABLED
    };
    for newline in ["\n", "\r\n"] {
        for operand in [
            "void",
            "_Bool",
            "_Complex double",
            "_Imaginary double",
            "_Atomic(int)",
            "const int",
            "volatile int",
            "restrict P",
            "struct S",
            "union U",
            "enum E",
        ] {
            let source =
                format!("#define MARKER\nint f(void) {{\n MARKER\n ({operand}) leaf(1);\n}}\n")
                    .replace('\n', newline);
            assert_eq!(
                scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
                scan_with(source.as_bytes(), Mode::C, without),
                "{source}"
            );
        }
    }
}

#[test]
fn split_calls_bound_the_original_gap_and_preserve_other_marks_and_text() {
    let without = Corrections {
        split_calls: false,
        ..Corrections::ENABLED
    };
    let candidate = Corrections::parse("enabled,split-calls").unwrap();
    for newline in ["\n", "\r\n"] {
        for gap in [
            "\n ",
            " /* before */\n /* after */ ",
            " /* across\n lines */ ",
        ] {
            for statement in [
                format!("return leaf{gap}(1);"),
                format!("int local = leaf{gap}(1);"),
                format!("int bound[leaf{gap}(1)];"),
                format!("return outer(leaf{gap}(1));"),
            ] {
                let source = format!("void f(void) {{ {statement} }}\n").replace('\n', newline);
                let mut records = scan_with(source.as_bytes(), Mode::C, candidate);
                let mut calls = 0;
                for record in &mut records {
                    for symbol in &mut record.symbols {
                        if symbol.mark == crate::unit::Mark::Call
                            && &record.text[symbol.at..symbol.at + symbol.len] == b"leaf"
                        {
                            symbol.mark = crate::unit::Mark::Plain;
                            calls += 1;
                        }
                    }
                }
                assert_eq!(calls, 1, "{source}");
                assert_eq!(
                    records,
                    scan_with(source.as_bytes(), Mode::C, without),
                    "{source}"
                );
            }
        }
    }
    for (gap_bytes, expected_calls) in [(253, 1), (255, 0), (256, 0), (1024, 0)] {
        let source = format!(
            "void f(void) {{ return leaf\n{}(1); }}\n",
            " ".repeat(gap_bytes - 1)
        );
        let records = scan_with(source.as_bytes(), Mode::C, candidate);
        assert_eq!(
            records
                .iter()
                .flat_map(|record| &record.symbols)
                .filter(|symbol| symbol.mark == crate::unit::Mark::Call)
                .count(),
            expected_calls
        );
    }
}

fn assigned_names(source: &[u8], corrections: Corrections) -> Vec<(u32, Vec<u8>)> {
    scan_with(source, Mode::C, corrections)
        .iter()
        .flat_map(|record| {
            record
                .symbols
                .iter()
                .filter(|symbol| symbol.assigned)
                .map(|symbol| {
                    (
                        record.line,
                        record.text[symbol.at..symbol.at + symbol.len].to_vec(),
                    )
                })
        })
        .collect()
}

#[test]
fn written_assignments_survive_clipped_earlier_comments_and_literals() {
    for newline in ["\n", "\r\n"] {
        for padding in [
            format!("/* {} */", "x".repeat(400)),
            format!("const char *padding = \"{}\";", "x".repeat(400)),
        ] {
            let source = format!(
                "{padding}\nconst signed char table[2] = {{1, 2}};\n\
                 void writes(int *slots) {{\n {padding}\n int counter = 0;\n\
                 counter++;\n --counter;\n slots[++counter] = 1;\n\
                 counter; counter++;\n counter += 1;\n}}\n\
                 void following(int *slots) {{\n counter++;\n slots[counter] = 1;\n}}\n"
            )
            .replace('\n', newline);
            assert_eq!(
                assigned_names(source.as_bytes(), Corrections::ENABLED)
                    .into_iter()
                    .filter(|(_, name)| name != b"padding")
                    .collect::<Vec<_>>(),
                vec![
                    (2, b"table".to_vec()),
                    (5, b"counter".to_vec()),
                    (6, b"counter".to_vec()),
                    (7, b"counter".to_vec()),
                    (8, b"slots".to_vec()),
                    (8, b"counter".to_vec()),
                    (9, b"counter".to_vec()),
                    (10, b"counter".to_vec()),
                    (13, b"counter".to_vec()),
                    (14, b"slots".to_vec()),
                ],
                "{newline:?} {padding}"
            );
        }
    }
}

#[test]
fn clipped_assignment_windows_keep_target_boundaries_and_independent_switches() {
    for newline in ["\n", "\r\n"] {
        let source = format!(
            "void writes(int *slots) {{\n /* {} */\n\
             ++(counter);\n ((counter))--;\n slots[++counter] = 1;\n\
             object.counter++; pointer->counter++; ++*pointer; (*pointer)++;\n\
             object.slots[0] = 1; *slots[0] = 1; (slots)[0] = 1;\n\
             slots[(0] = 1; slots[0] == 1; slots[] = 1;\n\
             counter + /* gap */ +other;\n}}\n",
            "\" ; { } [ ] /* ".repeat(30)
        )
        .replace('\n', newline);
        for (subscripts, mutations, expected) in [
            (false, false, vec![]),
            (true, false, vec![(5, b"slots".to_vec())]),
            (
                false,
                true,
                vec![
                    (3, b"counter".to_vec()),
                    (4, b"counter".to_vec()),
                    (5, b"counter".to_vec()),
                ],
            ),
            (
                true,
                true,
                vec![
                    (3, b"counter".to_vec()),
                    (4, b"counter".to_vec()),
                    (5, b"slots".to_vec()),
                    (5, b"counter".to_vec()),
                ],
            ),
        ] {
            let corrections = Corrections {
                subscript_assignments: subscripts,
                named_mutations: mutations,
                ..Corrections::ENABLED
            };
            assert_eq!(assigned_names(source.as_bytes(), corrections), expected);
            let without_assignments = |corrections| {
                let mut records = scan_with(source.as_bytes(), Mode::C, corrections);
                for record in &mut records {
                    for symbol in &mut record.symbols {
                        symbol.assigned = false;
                    }
                }
                records
            };
            assert_eq!(
                without_assignments(corrections),
                without_assignments(Corrections::ENABLED)
            );
        }
        for target in ["counter++;", "(counter)++;", "slots[0] = 1;"] {
            let source = format!(
                "void f(void) {{ object. /* {} */{target} }}\n",
                "x".repeat(400)
            );
            assert!(
                assigned_names(source.as_bytes(), Corrections::ENABLED).is_empty(),
                "{source}"
            );
        }
    }
}

#[test]
fn clipped_assignment_prefixes_respect_splices_and_escaped_literal_delimiters() {
    for newline in ["\n", "\r\n"] {
        for padding in [
            format!("/\\\n* {} *\\\n/", "x".repeat(400)),
            format!("// {}\\\n still comment\n", "x".repeat(400)),
            format!(
                "const char *padding = \"{}\\\" /* \\\n end\";",
                "x".repeat(400)
            ),
        ] {
            let source = format!(
                "void f(int *slots) {{\n {padding}\n ; counter++;\n slots[++counter] = 1;\n}}\n"
            )
            .replace('\n', newline);
            let names: Vec<_> = assigned_names(source.as_bytes(), Corrections::ENABLED)
                .into_iter()
                .map(|(_, name)| name)
                .filter(|name| name != b"padding")
                .collect();
            assert_eq!(
                names,
                [b"counter".to_vec(), b"slots".to_vec(), b"counter".to_vec()],
                "{source}"
            );
        }
    }
}

#[test]
fn written_assignments_reject_comments_formed_by_line_splices() {
    for newline in ["\n", "\r\n"] {
        let source = concat!(
            "void f(void) {\n",
            " /\\\n* counter++;\n slots[0] = 1; */\n",
            " /\\\n/ counter++; slots[0] = 1;\n",
            " counter /\\\n* gap *\\\n/ ++;\n",
            " slots /\\\n* gap *\\\n/ [0] = 1;\n",
            "}\n"
        )
        .replace('\n', newline);
        let names = assigned_names(source.as_bytes(), Corrections::ENABLED);
        assert_eq!(
            names,
            vec![(7, b"counter".to_vec()), (10, b"slots".to_vec())],
            "{newline:?}"
        );
    }
}

#[test]
fn named_mutations_mark_each_direct_written_operator() {
    let source = b"void step(void) {\n counter++;\n counter--;\n ++counter;\n --counter;\n counter += 1;\n counter = counter + 1;\n}\n";
    assert_eq!(
        assigned_names(source, Corrections::ENABLED),
        (2..=7)
            .map(|line| (line, b"counter".to_vec()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        assigned_names(source, Corrections::default()),
        vec![(6, b"counter".to_vec()), (7, b"counter".to_vec())]
    );
}

#[test]
fn named_mutations_accept_only_direct_name_operands() {
    let source = b"void step(void) {\n ++(counter);\n ((counter))--;\n pointer++;\n *pointer++;\n ++*pointer; (*pointer)++;\n object.counter++; pointer->counter++; slots[index]++;\n ++object.counter; ++pointer->counter; ++slots[index];\n ((int)counter)++;\n slots[counter++] = value;\n}\n";
    let corrections = Corrections {
        named_mutations: true,
        ..Corrections::default()
    };
    assert_eq!(
        assigned_names(source, corrections),
        vec![
            (2, b"counter".to_vec()),
            (3, b"counter".to_vec()),
            (4, b"pointer".to_vec()),
            (5, b"pointer".to_vec()),
            (10, b"counter".to_vec())
        ]
    );
    let texts = |corrections| {
        scan_with(source, Mode::C, corrections)
            .into_iter()
            .map(|record| (record.line, record.text))
            .collect::<Vec<_>>()
    };
    assert_eq!(texts(corrections), texts(Corrections::default()));
}

#[test]
fn named_mutations_obey_original_token_and_window_boundaries() {
    let source = "#define STEP ++counter\nvoid step(void) {\n ++ /* gap */ (counter);\n ( /* gap */ counter /* gap */ )--;\n other+++counter;\n other---counter;\n counter +/* gap */+other;\n counter -/* gap */-other;\n +\\\n+counter;\n counter+\\\n+;\n object-\\\n>counter++;\n counter // gap\n ++;\n ++ // gap\n counter;\n \"counter++\"; /* ++counter */\n}\n";
    for newline in ["\n", "\r\n"] {
        assert_eq!(
            assigned_names(
                source.replace('\n', newline).as_bytes(),
                Corrections::ENABLED
            ),
            vec![
                (1, b"counter".to_vec()),
                (3, b"counter".to_vec()),
                (4, b"counter".to_vec()),
                (5, b"other".to_vec()),
                (6, b"other".to_vec()),
                (10, b"counter".to_vec()),
                (11, b"counter".to_vec()),
                (15, b"counter".to_vec()),
                (18, b"counter".to_vec())
            ],
            "{newline:?}"
        );
    }
    let truncated_token = format!("other+++{}counter;\n", " ".repeat(254));
    assert_eq!(
        assigned_names(truncated_token.as_bytes(), Corrections::ENABLED),
        vec![(1, b"other".to_vec())]
    );
    assert_eq!(
        assigned_names(b"counter++;\n", Corrections::ENABLED),
        vec![(1, b"counter".to_vec())]
    );
    for source in [
        format!("void f(void) {{ object.{}counter++; }}\n", " ".repeat(256)),
        format!("void f(void) {{ sink({}counter)++; }}\n", " ".repeat(255)),
        format!("void f(void) {{ ++counter{}.member; }}\n", " ".repeat(256)),
    ] {
        assert!(assigned_names(source.as_bytes(), Corrections::ENABLED).is_empty());
    }
}

#[test]
fn named_mutations_preserve_written_operands_later_writes_and_definition_work() {
    let source = b"#define STEP ++counter\nvoid step(void) {\n sizeof(counter++);\n WRAPPER(--counter);\n for (; counter < limit; ++counter) next();\n if (counter-- < limit) use(counter);\n sink(++counter); value = counter++;\n (void)counter++; (int)(counter++);\n read(counter); counter++;\n counter + 1; sizeof(counter);\n}\n";
    assert_eq!(
        assigned_names(source, Corrections::ENABLED),
        vec![
            (1, b"counter".to_vec()),
            (3, b"counter".to_vec()),
            (4, b"counter".to_vec()),
            (5, b"counter".to_vec()),
            (6, b"counter".to_vec()),
            (7, b"counter".to_vec()),
            (7, b"value".to_vec()),
            (7, b"counter".to_vec()),
            (8, b"counter".to_vec()),
            (8, b"counter".to_vec()),
            (9, b"counter".to_vec())
        ]
    );
    let stress = format!(
        "void f(void) {{ {} }}\nint later({}) {{ return 1; }}\n",
        "read(counter); counter++; ".repeat(4000),
        "int parameter".to_owned() + &" ".repeat(1000)
    );
    let mut without = Corrections::ENABLED;
    without.named_mutations = false;
    let (before, before_cost) = super::scan_with_cost(stress.as_bytes(), Mode::C, without);
    let (after, after_cost) =
        super::scan_with_cost(stress.as_bytes(), Mode::C, Corrections::ENABLED);
    assert_eq!(before_cost.work, after_cost.work);
    assert_eq!(before_cost.narrow_windows, after_cost.narrow_windows);
    assert_eq!(before[1].symbols[0].mark, crate::unit::Mark::Function);
    assert_eq!(after[1].symbols[0].mark, crate::unit::Mark::Function);
    assert_eq!(
        assigned_names(stress.as_bytes(), Corrections::ENABLED).len(),
        4000
    );
}

#[test]
fn named_mutations_preserve_probe_text_and_other_navigation_marks() {
    let source = include_bytes!("../../tests/fixtures/scanner/probes/72-named-mutations.c");
    let mut without = Corrections::ENABLED;
    without.named_mutations = false;
    let before = scan_with(source, Mode::C, without);
    let after = scan_with(source, Mode::C, Corrections::ENABLED);
    let text = |records: &[Record]| {
        records
            .iter()
            .map(|record| (record.line, record.text.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(text(&before), text(&after));
    let marked = |records: &[Record]| {
        records
            .iter()
            .flat_map(|record| {
                record
                    .symbols
                    .iter()
                    .filter(|symbol| {
                        // An end at the final name may use the record's define_end_after flag.
                        !matches!(
                            symbol.mark,
                            crate::unit::Mark::Plain | crate::unit::Mark::DefineEnd
                        )
                    })
                    .map(|symbol| {
                        (
                            record.line,
                            symbol.mark,
                            record.text[symbol.at..symbol.at + symbol.len].to_vec(),
                        )
                    })
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(marked(&before), marked(&after));
}

#[test]
fn subscript_assignments_mark_initializers_and_written_stores() {
    let source = b"int slots[2] = {0};\nvoid set(void) {\n slots[0] = 1;\n slots[1] += 1;\n if (slots[0] == 1) other();\n}\n";
    assert_eq!(
        assigned_names(source, Corrections::ENABLED),
        vec![
            (1, b"slots".to_vec()),
            (3, b"slots".to_vec()),
            (4, b"slots".to_vec())
        ]
    );
    assert!(assigned_names(source, Corrections::default()).is_empty());
}

#[test]
fn subscript_assignments_distinguish_declarators_from_indirect_stores() {
    let source = b"int *slots[4] = {0};\nchar text[] = \"ok\";\nvoid f(int *pointer) {\n int local[] = {1};\n pointer[0] = 1;\n *pointer[0] = 2;\n pointer[] = 3;\n}\n";
    assert_eq!(
        assigned_names(source, Corrections::ENABLED),
        vec![
            (1, b"slots".to_vec()),
            (2, b"text".to_vec()),
            (4, b"local".to_vec()),
            (5, b"pointer".to_vec())
        ]
    );
}

#[test]
fn subscript_assignments_preserve_nested_writes_calls_and_later_occurrences() {
    let source = b"void f(void) {\n slots[0]; slots[1] = 1; slots[2] += 2;\n index; slots[index = 1] = source[index];\n slots[other[next()] = 1] = value;\n slots[other[0]] == value;\n scalar; scalar = 1;\n}\n";
    assert_eq!(
        assigned_names(source, Corrections::ENABLED),
        vec![
            (2, b"slots".to_vec()),
            (2, b"slots".to_vec()),
            (3, b"slots".to_vec()),
            (3, b"index".to_vec()),
            (4, b"slots".to_vec()),
            (4, b"other".to_vec())
        ]
    );
    let records = scan_with(source, Mode::C, Corrections::ENABLED);
    let nested = records.iter().find(|record| record.line == 4).unwrap();
    assert!(
        nested
            .symbols
            .iter()
            .any(|symbol| symbol.mark == crate::unit::Mark::Call
                && &nested.text[symbol.at..symbol.at + symbol.len] == b"next")
    );
}

#[test]
fn subscript_assignments_keep_text_and_exclude_other_lvalues() {
    let source = b"void f(void) {\n slots[0]; return slots[1] /* gap */ = 1;\n slots[0]; if (slots[1] += 2) next();\n object.slots[0] = 1; pointer->slots[0] = 2;\n object. /* gap */\n slots[0] = 3;\n (slots)[0] = 1; (*slots)[0] = 2; factory()[0] = 3;\n slots[0].member = 1; slots[0]++; --slots[0];\n slots[0] == 1; slots[0] != 1; slots[0] <= 1; slots[0] >= 1;\n object. // gap\n slots[0] = 4;\n}\n";
    let baseline = scan_with(source, Mode::C, Corrections::default());
    let corrected = scan_with(
        source,
        Mode::C,
        Corrections {
            subscript_assignments: true,
            ..Corrections::default()
        },
    );
    assert_eq!(
        baseline
            .iter()
            .map(|record| (&record.line, &record.text))
            .collect::<Vec<_>>(),
        corrected
            .iter()
            .map(|record| (&record.line, &record.text))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        assigned_names(source, Corrections::ENABLED),
        vec![
            (2, b"slots".to_vec()),
            (3, b"slots".to_vec()),
            (8, b"member".to_vec())
        ]
    );
}

#[test]
fn subscript_assignments_obey_original_byte_window_and_lexical_boundaries() {
    let mut source = b"void f(void) {\n slots[".to_vec();
    source.extend(std::iter::repeat_n(b' ', 250));
    source.extend_from_slice(b"0] = 1;\n slots[");
    source.extend(std::iter::repeat_n(b' ', 251));
    source.extend_from_slice(b"0] == 1;\n slots[");
    source.extend(std::iter::repeat_n(b' ', 253));
    source.extend_from_slice(b"0] = 1;\n slots[call(\"]\", '\\'') /* ] */] <<= 1;\n slots\\\r\n [0]\r\n = 1;\n slots[0; next()] = 1;\n slots[(0] = 1;\n slots[0] /* unclosed\n}");
    assert_eq!(
        assigned_names(&source, Corrections::ENABLED),
        vec![
            (2, b"slots".to_vec()),
            (5, b"slots".to_vec()),
            (6, b"slots".to_vec())
        ]
    );
    for suffix in [
        b"[0] #define X = 1".as_slice(),
        b"[0] / * = 1",
        b"[0] + = 1",
        b"[0] << = 1",
        b"[\"unclosed] = 1",
        b"[0] //\\\n [0] = 1\n",
        b"[0] { = 1",
        b"[] = 1",
    ] {
        let mut source = b"void f(void) { slots".to_vec();
        source.extend_from_slice(suffix);
        assert!(
            !assigned_names(&source, Corrections::ENABLED)
                .iter()
                .any(|(_, name)| name == b"slots"),
            "{suffix:?}"
        );
    }
}

#[test]
fn subscript_assignments_read_spliced_operators_without_joining_comments() {
    for newline in [b"\n".as_slice(), b"\r\n"] {
        let source = b"void f(void) {\n slots[0] =\\\n= 1;\n slots[0] +\\\n= 1;\n pointer-\\\n>slots[0] = 1;\n slots[0] <\\\n<\\\n= 1;\n slots[0] +/* gap */= 1;\n}\n";
        let source = source
            .split(|byte| *byte == b'\n')
            .collect::<Vec<_>>()
            .join(newline);
        assert_eq!(
            assigned_names(&source, Corrections::ENABLED),
            vec![(4, b"slots".to_vec()), (8, b"slots".to_vec())]
        );
    }
}

#[test]
fn subscript_assignments_cover_each_written_operator_with_an_independent_budget() {
    let mut source = b"void stores(void) {\n".to_vec();
    for operator in [
        "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=",
    ] {
        source.extend_from_slice(format!("slots[0] {operator} value;\n").as_bytes());
    }
    source.extend_from_slice(b"}\n");
    assert_eq!(
        assigned_names(&source, Corrections::ENABLED),
        (2..=12)
            .map(|line| (line, b"slots".to_vec()))
            .collect::<Vec<_>>()
    );
    let stress = format!(
        "void stress(void) {{ {} }}\nint later({}void) {{ return 1; }}\n",
        "slots[other[next()]] = value; ".repeat(4_000),
        " ".repeat(1_024)
    );
    let mut without = Corrections::ENABLED;
    without.subscript_assignments = false;
    let (before, before_cost) = super::scan_with_cost(stress.as_bytes(), Mode::C, without);
    let (after, after_cost) =
        super::scan_with_cost(stress.as_bytes(), Mode::C, Corrections::ENABLED);
    assert_eq!(
        before.last(),
        after.last(),
        "element stores must not narrow the later definition window"
    );
    assert_eq!(before_cost.work, after_cost.work);
    assert_eq!(before_cost.narrow_windows, after_cost.narrow_windows);
}

/// A fixture string: each character is one byte of the reference's decoded database.
fn bytes_of(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .expect("fixture text")
        .chars()
        .map(|character| u8::try_from(u32::from(character)).expect("fixture bytes are 0-255"))
        .collect()
}

/// One record in the fixture schema: line, fragments, (mark, name) pairs and macro end.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    line: u64,
    fragments: Vec<Vec<u8>>,
    symbols: Vec<(u8, Vec<u8>, bool)>,
    define_end: bool,
}

fn shape_of(record: &Record) -> Shape {
    let mut fragments = Vec::new();
    let mut symbols = Vec::new();
    let mut from = 0;
    for symbol in &record.symbols {
        fragments.push(reference_view(&record.text[from..symbol.at]));
        symbols.push((
            symbol.mark.byte(),
            reference_view(&record.text[symbol.at..symbol.at + symbol.len]),
            symbol.assigned,
        ));
        from = symbol.at + symbol.len;
    }
    fragments.push(reference_view(&record.text[from..]));
    Shape {
        line: u64::from(record.line),
        fragments,
        symbols,
        define_end: record.define_end_after,
    }
}

fn fixture_shape(record: &Value) -> Shape {
    Shape {
        line: record["line"].as_u64().expect("line"),
        fragments: record["fragments"]
            .as_array()
            .expect("fragments")
            .iter()
            .map(bytes_of)
            .collect(),
        symbols: record["symbols"]
            .as_array()
            .expect("symbols")
            .iter()
            .map(|triple| {
                (
                    bytes_of(&triple[0])[0],
                    bytes_of(&triple[1]),
                    triple[2].as_bool().expect("assigned"),
                )
            })
            .collect(),
        define_end: record["define_end"].as_bool().expect("define_end"),
    }
}

fn show(shape: &Shape) -> String {
    let mut text = Vec::new();
    for (index, fragment) in shape.fragments.iter().enumerate() {
        text.extend_from_slice(fragment);
        if let Some((mark, name, assigned)) = shape.symbols.get(index) {
            text.extend_from_slice(b"[");
            text.push(*mark);
            text.extend_from_slice(name);
            if *assigned {
                text.extend_from_slice(b" =");
            }
            text.extend_from_slice(b"]");
        }
    }
    format!(
        "{} {}{}",
        shape.line,
        String::from_utf8_lossy(&text),
        if shape.define_end { " +)" } else { "" }
    )
}

/// A probe's fixture in `kind` (`expected`: the reference's records; `intended`: the
/// enabled reading's, where it differs).
fn fixture_path(probe: &Path, kind: &str) -> std::path::PathBuf {
    probe
        .parent()
        .and_then(Path::parent)
        .expect("fixture root")
        .join(kind)
        .join(format!(
            "{}.json",
            probe.file_stem().expect("stem").to_string_lossy()
        ))
}

/// Differences between the scanner, with `corrections`, and one of a probe's fixtures,
/// one line each.
fn probe_differences(probe: &Path, fixture: &Path, corrections: Corrections) -> Vec<String> {
    let name = probe
        .file_name()
        .expect("probe name")
        .to_string_lossy()
        .into_owned();
    let bytes = std::fs::read(probe).expect("probe bytes");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(fixture).expect("fixture"))
        .expect("fixture JSON");
    let expected: Vec<Shape> = fixture["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(fixture_shape)
        .collect();
    let actual: Vec<Shape> = scan_with(&bytes, Mode::for_name(name.as_bytes()), corrections)
        .iter()
        .map(shape_of)
        .collect();
    let mut differences = Vec::new();
    let longest = expected.len().max(actual.len());
    for index in 0..longest {
        match (expected.get(index), actual.get(index)) {
            (Some(want), Some(got)) if want == got => {}
            (want, got) => differences.push(format!(
                "{name} #{index}\n  want {}\n  got  {}",
                want.map_or_else(|| "(none)".into(), show),
                got.map_or_else(|| "(none)".into(), show)
            )),
        }
    }
    differences
}

#[test]
fn scanner_matches_every_reference_probe() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner/probes");
    let mut entries: Vec<_> = std::fs::read_dir(probes)
        .expect("probe directory")
        .map(|entry| entry.expect("probe entry").path())
        .collect();
    entries.sort();
    let mut report = String::new();
    let mut failing = 0;
    for probe in &entries {
        let differences = probe_differences(
            probe,
            &fixture_path(probe, "expected"),
            Corrections::default(),
        );
        if !differences.is_empty() {
            failing += 1;
            for difference in differences.iter().take(6) {
                let _ = writeln!(report, "{difference}");
            }
        }
    }
    assert!(
        failing == 0,
        "{failing} of {} probes differ:\n{report}",
        entries.len()
    );
}

/// `#define` names longer than 250 bytes, which the reference cuts to 250 while its symbol
/// runs on over the text after the name, and one of 249 bytes, after which it stores a NUL
/// (reading-c.md §11). The baseline keeps every name whole and the line as written, where
/// Ripscope defines its own behaviour (ADR 0004). The open fixture keeps the reference's
/// records for it. Every other line reads as the reference's.
#[test]
fn long_define_names_are_stored_whole() {
    let probe = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scanner/open/probes/long-define-names.c");
    let source = std::fs::read(&probe).expect("probe");
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(fixture_path(&probe, "expected")).expect("fixture"))
            .expect("fixture JSON");
    let reference: Vec<Shape> = fixture["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(fixture_shape)
        .collect();
    let baseline: Vec<Shape> = scan(&source, Mode::C).iter().map(shape_of).collect();
    let lines: Vec<&[u8]> = source.split(|byte| *byte == b'\n').collect();
    assert_eq!(reference.len(), baseline.len());
    let (mut long, mut nul) = (0, 0);
    for (want, got) in reference.iter().zip(&baseline) {
        assert_eq!(want.line, got.line);
        let line = lines[usize::try_from(want.line).expect("line") - 1];
        let Some(rest) = line.strip_prefix(b"#define").map(|rest| &rest[1..]) else {
            assert_eq!(want, got, "{}", show(got));
            continue;
        };
        let length = rest
            .iter()
            .position(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_'))
            .unwrap_or(rest.len());
        let name = &rest[..length];
        assert_eq!(got.symbols[0].1, name, "{}", show(got));
        // The rest of the record is the line as written: the reading of the same line with
        // a short name in its place.
        let short = [&line[..b"#define ".len()], b"S", &rest[length..], b"\n"].concat();
        let short = shape_of(&scan(&short, Mode::C)[0]);
        assert_eq!(
            (&got.fragments, &got.symbols[1..], got.define_end),
            (&short.fragments, &short.symbols[1..], short.define_end),
            "{}",
            show(got)
        );
        assert_eq!(got.symbols[0].0, short.symbols[0].0, "{}", show(got));
        if length > 250 {
            let after = &rest[length..];
            let taken = &after[..(length - 250).min(after.len())];
            assert_eq!(
                want.symbols[0].1,
                [&name[..250], taken].concat(),
                "{}",
                show(want)
            );
            long += 1;
        } else {
            // A name that passes the limit short of 250 bytes keeps its end, and the
            // reference stores a NUL after it.
            assert_eq!(length, 249, "{}", show(got));
            assert_eq!(want.symbols[0].1, name, "{}", show(want));
            assert_eq!(want.fragments[1].first(), Some(&0), "{}", show(want));
            nul += 1;
        }
    }
    assert_eq!((long, nul), (6, 1));
}

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

/// Token soup: fragments of C, preprocessor lines, lex and yacc markers, and raw bytes.
const SOUP: &[&[u8]] = &[
    b"a",
    b"bb",
    b"_c",
    b"$d",
    b"int",
    b"struct",
    b"enum",
    b"union",
    b"class",
    b"typedef",
    b"extern",
    b"if",
    b"else",
    b"return",
    b"_Generic",
    b"_Alignof",
    b"(",
    b")",
    b"{",
    b"}",
    b"[",
    b"]",
    b";",
    b",",
    b"=",
    b"==",
    b"+=",
    b"<<=",
    b":",
    b"*",
    b"&",
    b".",
    b"1",
    b"0x1F",
    b"'x'",
    b"\"s\"",
    b"\"",
    b"'",
    b"\\",
    b"\\n",
    b" ",
    b"  ",
    b"\t",
    b"\r",
    b"\x0b",
    b"\x0c",
    b"\n",
    b"\n",
    b"/*",
    b"*/",
    b"**/",
    b"***/",
    b"//",
    b"#",
    b"#define ",
    b"#include ",
    b"<",
    b">",
    b"#if ",
    b"#ifdef ",
    b"#else\n",
    b"#elif ",
    b"#endif\n",
    b"%{",
    b"%}",
    b"%%",
    b"\0",
    b"\x01",
    b"\x7f",
    b"\xc3\xa9",
    b"\xff",
    b"x/../y",
    b"//////",
];

/// Any bytes give ordered, in-bounds records that form a unit.
#[test]
fn scanning_is_total() {
    let reproducers: [&[u8]; 4] = [
        b"a < bbbbbbbb; #include \"x/../y.h\" b",
        b"a < bbbbbbbb; #include \"x/../y.h\"",
        b"x = \"////////////////////////\"; yyyyyyyy #include \"q\" yyyy",
        // A name that starts before a rewritten include operand and ends inside it.
        b"''<#include\".h\"x/../i#include\"h\"",
    ];
    let mut random = Random(0x0dd_ba11_cafe_f00d);
    let generated = (0..10_000).map(|_| {
        let pieces = 1 + random.below(60);
        (0..pieces)
            .flat_map(|_| SOUP[random.below(SOUP.len())].iter().copied())
            .collect::<Vec<u8>>()
    });
    let inputs: Vec<Vec<u8>> = reproducers
        .iter()
        .map(|bytes| bytes.to_vec())
        .chain(generated)
        .collect();
    for (index, bytes) in inputs.iter().enumerate() {
        for mode in [
            Mode::C,
            Mode::Lex,
            Mode::Yacc,
            Mode::StateDescription,
            Mode::Breakpoints,
        ] {
            for corrections in [Corrections::default(), Corrections::ENABLED] {
                let records = scan_with(bytes, mode, corrections);
                let mut line = 0;
                for record in &records {
                    assert!(record.line >= line, "input {index} {mode:?}: lines go back");
                    line = record.line;
                    let mut at = 0;
                    for symbol in &record.symbols {
                        assert!(
                            symbol.at >= at && symbol.at + symbol.len <= record.text.len(),
                            "input {index} {mode:?}: name out of order or bounds in {:?}",
                            String::from_utf8_lossy(bytes)
                        );
                        at = symbol.at;
                    }
                }
                crate::unit::AnalysisUnit::from_records(&records).unwrap_or_else(|error| {
                    panic!("input {index} {mode:?}: {error}");
                });
            }
        }
    }
}

/// The enabled Corrections. A change to the set follows ADR 0002 and updates this list.
#[test]
fn the_enabled_set_is_pinned() {
    assert_eq!(
        Corrections::ENABLED.names(),
        "linkage-blocks,definition-bodies,directive-continuations,crlf-continuations,\
         unspaced-conditionals,macro-bodies,initializer-entries,definition-heads,\
         macro-line-comments,pointer-declarators,parenthesised-typedefs,typedef-qualifiers,\
         extern-definitions,typedef-parameters,attribute-expressions,conditional-comments,keyword-macros,\
         bare-directive-words,opaque-member-tails,typedef-decorations,dereferenced-calls,c11-expression-keywords,atomic-types,preprocessor-defined,macro-function-heads,subscript-assignments,named-mutations,split-calls,closing-arms,written-head-names,conditional-lists,alternative-bodies"
    );
    assert_eq!(Corrections::parse("enabled"), Ok(Corrections::ENABLED));
    assert_eq!(
        Corrections::parse(&Corrections::ENABLED.names()),
        Ok(Corrections::ENABLED)
    );
    assert_eq!(Corrections::default().names(), "");
}

/// Every Correction changes some probe's records, so the version digest sees a change
/// to any Correction's rule, and each has a probe fixture showing both readings.
#[test]
fn every_correction_changes_some_probe() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner/probes");
    let mut entries: Vec<_> = std::fs::read_dir(probes)
        .expect("probe directory")
        .map(|entry| entry.expect("probe entry").path())
        .collect();
    entries.sort();
    let sources: Vec<(Mode, Vec<u8>)> = entries
        .iter()
        .map(|probe| {
            let name = probe
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            (
                Mode::for_name(name.as_bytes()),
                std::fs::read(probe).expect("probe"),
            )
        })
        .collect();
    let names = Corrections::ENABLED.names();
    for name in names.split(',') {
        let alone = Corrections::parse(name).expect("known");
        let acts = sources
            .iter()
            .any(|(mode, bytes)| scan_with(bytes, *mode, alone) != scan(bytes, *mode));
        assert!(acts, "no probe shows what {name} does: add one");
        // Within the enabled set too, which the digest and the index depend on.
        let others: Vec<&str> = names.split(',').filter(|other| *other != name).collect();
        let without = Corrections::parse(&others.join(",")).expect("known");
        let acts = sources.iter().any(|(mode, bytes)| {
            scan_with(bytes, *mode, without) != scan_with(bytes, *mode, Corrections::ENABLED)
        });
        assert!(
            acts,
            "no probe shows what {name} adds to the enabled set: add one"
        );
    }
}

/// The scanner's records for every probe, read as the reference reads it and with the
/// enabled Corrections, digested. When this changes because the scanner reads something
/// differently, bump `VERSION` so existing indexes analyse again; a new probe changes it
/// without a new version. Then update the digest here.
#[test]
fn probe_records_match_the_scanner_version() {
    use sha2::{Digest, Sha256};
    const VERSION_38_DIGEST: &str =
        "2287967afdec583095d9c80f1e7284a7454d243a6a38fee1549154a7ac836e57";
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner/probes");
    let mut entries: Vec<_> = std::fs::read_dir(probes)
        .expect("probe directory")
        .map(|entry| entry.expect("probe entry").path())
        .collect();
    entries.sort();
    let mut hasher = Sha256::new();
    for probe in &entries {
        let name = probe
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let bytes = std::fs::read(probe).expect("probe");
        for corrections in [Corrections::default(), Corrections::ENABLED] {
            for record in scan_with(&bytes, Mode::for_name(name.as_bytes()), corrections) {
                hasher.update(format!("{name} {record:?}\n").as_bytes());
            }
        }
    }
    let digest = base16ct::lower::encode_string(&hasher.finalize());
    assert_eq!(
        super::VERSION,
        38,
        "update the digest's name with the version"
    );
    assert_eq!(
        digest, VERSION_38_DIGEST,
        "the scanner's records changed: bump scanner::VERSION and update the digest"
    );
}

#[test]
fn return_type_wrappers_keep_unresolved_heads_and_displayed_text() {
    let without = Corrections::ENABLED;
    let candidate = Corrections::parse("enabled,return-type-wrappers").expect("candidate reading");
    for source in [
        "RETTYPE(Node) opaque(void) { return 1; }\n".to_owned(),
        "int RETTYPE(void) SUFFIX(void) { return 1; }\n".to_owned(),
        "#define SUFFIX(arg)\nAPI(void) SUFFIX(void) { return 1; }\n".to_owned(),
        "RETTYPE(int value) SUFFIX(void) { return value; }\n".to_owned(),
        "RETTYPE() empty(void) { return 1; }\n".to_owned(),
        "RETTYPE(1) numeric(void) { return 1; }\n".to_owned(),
        "RETTYPE((int)) nested(void) { return 1; }\n".to_owned(),
        "RETTYPE(int) annotated(void) ATTR { return 1; }\n".to_owned(),
        "RETTYPE(int) ANNOTATED decorated(void) { return 1; }\n".to_owned(),
        "RETTYPE(int) callback_head(int callback(int)) { return callback(1); }\n".to_owned(),
        "#define WRAP(x) WRAP_##x\n#define WRAP_int\n#define ATTR\n\
         int WRAP(int retained_inner(int value)) ATTR { return value; }\n"
            .to_owned(),
        "RETTYPE(int)\n#if FLAG\nconditional(void) { return 1; }\n#endif\n".to_owned(),
        "RETTYPE(int) prototype(void);\nint after;\n".to_owned(),
        "struct S { RETTYPE(int) nested(void); };\n".to_owned(),
        format!(
            "RETTYPE(int) /*{}*/ distant(void) {{ return 1; }}\n",
            " ".repeat(256)
        ),
    ] {
        assert_eq!(
            scan_with(source.as_bytes(), Mode::C, candidate),
            scan_with(source.as_bytes(), Mode::C, without),
            "unresolved wrapper shape: {source:?}"
        );
    }
    let source = include_str!("../../tests/fixtures/scanner/probes/70-return-type-wrappers.c");
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        let before = scan_with(source.as_bytes(), Mode::C, without);
        let after = scan_with(source.as_bytes(), Mode::C, candidate);
        assert_eq!(
            before.iter().map(|r| (r.line, &r.text)).collect::<Vec<_>>(),
            after.iter().map(|r| (r.line, &r.text)).collect::<Vec<_>>(),
            "wrapper recognition must preserve physical lines and displayed text"
        );
    }
}

#[test]
fn wrapper_work_cannot_narrow_later_definition_recognition() {
    let source = format!(
        "{}\nint later({}void) {{ return 1; }}\n",
        "WRAPPER(int) ".repeat(30_000),
        " ".repeat(1_024),
    );
    let without = Corrections::ENABLED;
    let candidate = Corrections::parse("enabled,return-type-wrappers").expect("candidate reading");
    assert_eq!(
        scan_with(source.as_bytes(), Mode::C, candidate).last(),
        scan_with(source.as_bytes(), Mode::C, without).last(),
        "wrapper look-ahead must preserve the later ordinary function definition"
    );
}

#[test]
fn proposed_return_type_wrapper_reading_has_its_own_paired_fixture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner");
    let probe = root.join("probes/70-return-type-wrappers.c");
    let fixture = root.join("proposed/70-return-type-wrappers.json");
    let candidate = Corrections::parse("enabled,return-type-wrappers").expect("candidate reading");
    assert!(!Corrections::parse("enabled").unwrap().return_type_wrappers);
    assert_ne!(
        scan_with(&std::fs::read(&probe).expect("probe"), Mode::C, candidate),
        scan_with(
            &std::fs::read(&probe).expect("probe"),
            Mode::C,
            Corrections::ENABLED
        ),
        "the opt-in candidate must demonstrate its different reading"
    );
    let differences = probe_differences(&probe, &fixture, candidate);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    assert_eq!(
        marks(
            "#define GNU_EMPTY(arg)\ngnu_real(int) GNU_EMPTY(void) { return 1; }\n",
            "enabled"
        ),
        "1:#GNU_EMPTY, arg 2:$gnu_real,$GNU_EMPTY,}"
    );
    assert_eq!(
        marks(
            "#define GNU_EMPTY(arg)\ngnu_real(int) GNU_EMPTY(void) { return 1; }\n",
            "enabled,return-type-wrappers"
        ),
        "1:#GNU_EMPTY, arg 2: gnu_real,$GNU_EMPTY,}"
    );
}

#[test]
fn macro_function_heads_keep_their_physical_lines_with_both_line_endings() {
    let source = include_str!("../../tests/fixtures/scanner/probes/69-macro-function-heads.c");
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        let records = scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED);
        for line in [3, 8, 13, 14, 18] {
            let record = records
                .iter()
                .find(|record| record.line == line)
                .expect("head line");
            assert!(
                record.symbols.iter().any(|symbol| {
                    symbol.mark.byte() == b' '
                        && &record.text[symbol.at..symbol.at + symbol.len] == b"NAME"
                }),
                "line {line}: {record:?}"
            );
            assert!(
                !record.symbols.iter().any(|symbol| {
                    symbol.mark.byte() == b'`'
                        && &record.text[symbol.at..symbol.at + symbol.len] == b"NAME"
                }),
                "line {line}: {record:?}"
            );
        }
    }
}

#[test]
fn macro_head_work_cannot_narrow_later_definition_recognition() {
    let source = format!(
        "#define UNFINISHED(NAME) {}\nint later({}void) {{ return 1; }}\n",
        "int NAME(".repeat(4_000),
        " ".repeat(1_024),
    );
    let mut without = Corrections::ENABLED;
    without.macro_function_heads = false;
    assert_eq!(
        scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED).last(),
        scan_with(source.as_bytes(), Mode::C, without).last(),
        "macro look-ahead must preserve the later ordinary function definition"
    );
}

#[test]
fn macro_constructor_heads_change_only_the_initial_constructor_calls() {
    let source = include_str!("../../tests/fixtures/scanner/probes/73-macro-constructor-heads.c");
    for newline in ["\n", "\r\n"] {
        let source = source.replace('\n', newline);
        let enabled = scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED);
        let mut without = Corrections::ENABLED;
        without.macro_function_heads = false;
        let mut expected = scan_with(source.as_bytes(), Mode::C, without);
        for record in &mut expected {
            if record.line == 22 {
                record.symbols.retain(|symbol| {
                    symbol.mark != crate::unit::Mark::Call
                        || &record.text[symbol.at..symbol.at + symbol.len] != b"NAME"
                });
            }
            for symbol in &mut record.symbols {
                let name = &record.text[symbol.at..symbol.at + symbol.len];
                if symbol.mark == crate::unit::Mark::Call
                    && ((name == b"CppConcat" && [5, 11, 16, 19, 21].contains(&record.line))
                        || (name == b"Ctor" && record.line == 28))
                {
                    assert_eq!(symbol.mark, crate::unit::Mark::Call);
                    symbol.mark = crate::unit::Mark::Plain;
                }
            }
        }
        assert_eq!(enabled.len(), expected.len());
        for (actual, expected) in enabled.iter().zip(expected) {
            assert_eq!(actual, &expected, "{newline:?} line {}", actual.line);
        }
    }
}

#[test]
fn macro_constructor_heads_keep_invalid_prefixes_and_lists() {
    let mut without = Corrections::ENABLED;
    without.macro_function_heads = false;
    let mut cases: Vec<String> = [
        "",
        "static extern inline const volatile restrict ",
        "int * ",
        "struct tag ",
        "union tag ",
        "enum tag ",
        "int + ",
        "__attribute__((unused)) int ",
        "decorate() int ",
        "1 int ",
        "\"text\" int ",
        "#if FLAG int ",
        "int; Datum ",
        "int Datum ",
        "Datum Other ",
        "typedef int ",
        "_Bool ",
        "_Atomic ",
        "_Noreturn int ",
        "int /* unspliced\n*/ ",
    ]
    .into_iter()
    .map(|prefix| format!("{prefix}CppConcat(left, right)(void) {{ helper(); }}"))
    .collect();
    cases.push(format!(
        "int {}CppConcat(left, right)(void) {{ helper(); }}",
        " ".repeat(256)
    ));
    for tail in [
        "(left + more, right)(void) { helper(); }",
        "(left, right, extra)(void) { helper(); }",
        "((left), right)(void) { helper(); }",
        "(int, right)(void) { helper(); }",
        "(inline, right)(void) { helper(); }",
        "(left, restrict)(void) { helper(); }",
        "(left, _Bool)(void) { helper(); }",
        "(left, right)(void);",
        "(left, right)(void) SUFFIX { helper(); }",
        "(left, right)(void)",
        "(left, right)(next, layer)(void) { helper(); }",
    ] {
        cases.push(format!("int CppConcat{tail}"));
    }
    for case in cases {
        for newline in ["\n", "\r\n"] {
            let source = format!("#define CASE(stat) {case}\nint later(void) {{ helper(); }}\n")
                .replace('\n', newline);
            assert_eq!(
                scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
                scan_with(source.as_bytes(), Mode::C, without),
                "{case:?} {newline:?}"
            );
        }
    }
}

#[test]
fn macro_constructor_heads_bound_original_prefix_bytes_and_lookahead() {
    let mut without = Corrections::ENABLED;
    without.macro_function_heads = false;
    for newline in ["\n", "\r\n"] {
        for length in [256, 257] {
            let prefix = format!(
                " \\{newline} int /*{}*/ ",
                " ".repeat(length - newline.len() - 12)
            );
            assert_eq!(prefix.len(), length);
            let source = format!(
                "#define HEAD(stat){prefix}Ctor(left, right)(void) {{ helper(); }}{newline}"
            );
            let enabled = scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED);
            let disabled = scan_with(source.as_bytes(), Mode::C, without);
            assert_eq!(enabled == disabled, length == 257);
        }
    }
    for source in [
        format!(
            "#define HEAD(stat) int Ctor(left, right)({}void) {{ helper(); }}\n",
            " ".repeat(64 * 1024)
        ),
        format!(
            "#define HEAD(stat) int Ctor(left, right)(void) /*{}*/ {{ helper(); }}\n",
            " ".repeat(64 * 1024)
        ),
    ] {
        assert_eq!(
            scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
            scan_with(source.as_bytes(), Mode::C, without)
        );
    }
}

#[test]
fn macro_constructor_heads_start_after_spliced_macro_formals() {
    for newline in ["\n", "\r\n"] {
        for splices in [1, 3] {
            let splice = format!("\\{newline}").repeat(splices);
            let source = format!(
                "#define HEAD{splice}(stat) int Ctor(left, right)(void) {{ helper(); }}{newline}"
            );
            let roles = |corrections| {
                scan_with(source.as_bytes(), Mode::C, corrections)
                    .iter()
                    .flat_map(|record| {
                        record.symbols.iter().map(|symbol| {
                            (
                                record.text[symbol.at..symbol.at + symbol.len].to_vec(),
                                symbol.mark,
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            };
            let mut without = Corrections::ENABLED;
            without.macro_function_heads = false;
            assert!(roles(without).contains(&(b"Ctor".to_vec(), crate::unit::Mark::Call)));
            assert!(
                roles(Corrections::ENABLED).contains(&(b"Ctor".to_vec(), crate::unit::Mark::Plain))
            );
            assert!(
                roles(Corrections::ENABLED)
                    .contains(&(b"helper".to_vec(), crate::unit::Mark::Call))
            );
            for gap in [" ", "/* comment */"] {
                let source = format!(
                    "#define HEAD{splice}{gap}(stat) int Ctor(left, right)(void) {{ helper(); }}{newline}"
                );
                assert_eq!(
                    scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
                    scan_with(source.as_bytes(), Mode::C, without),
                    "a gap before '(' starts object replacement text"
                );
            }
        }
    }
}

#[test]
fn macro_function_heads_do_not_borrow_a_body_after_their_line_or_window() {
    let mut without = Corrections::ENABLED;
    without.macro_function_heads = false;
    for source in [
        "#define HEAD(NAME) static int NAME(void)\n{ helper(); }\n".to_owned(),
        "#define HEAD(NAME) static int NAME(void) /* comment\n*/ { helper(); }\n".to_owned(),
        "#define HEAD(NAME) static int NAME(void) // comment\n{ helper(); }\n".to_owned(),
        "#define HEAD(NAME) static int NAME(\"text\n\") { helper(); }\n".to_owned(),
        format!(
            "#define HEAD(NAME) static int NAME({}void) {{ helper(); }}\n",
            " ".repeat(64 * 1024)
        ),
        format!(
            "#define HEAD(NAME) static int NAME(void) /*{}*/ {{ helper(); }}\n",
            " ".repeat(64 * 1024)
        ),
    ] {
        assert_eq!(
            scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
            scan_with(source.as_bytes(), Mode::C, without),
            "an unfinished macro head must keep its reading"
        );
    }
}

#[test]
fn preprocessor_defined_tracks_continuations_with_both_line_endings() {
    let source = concat!(
        "int function(void) {\n",
        "#if FLAG && \\\n",
        "defined(OPTION) && nested(real())\n",
        "call();\n",
        "#endif\n",
        "return defined(2);\n",
        "}\n",
    );
    for source in [source.to_string(), source.replace('\n', "\r\n")] {
        let roles: Vec<_> = scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED)
            .into_iter()
            .flat_map(|record| {
                record.symbols.into_iter().filter_map(move |symbol| {
                    let name = record.text[symbol.at..symbol.at + symbol.len].to_vec();
                    (symbol.mark == crate::unit::Mark::Call || name == b"defined").then_some((
                        record.line,
                        symbol.mark.byte(),
                        name,
                    ))
                })
            })
            .collect();
        assert_eq!(
            roles,
            vec![
                (3, b' ', b"defined".to_vec()),
                (3, b'`', b"nested".to_vec()),
                (3, b'`', b"real".to_vec()),
                (4, b'`', b"call".to_vec()),
                (6, b'`', b"defined".to_vec()),
            ],
            "conditional and ordinary roles {source:?}"
        );
    }
}

#[test]
fn preprocessor_defined_preserves_nonconditional_readings() {
    let mut without = Corrections::ENABLED;
    without.preprocessor_defined = false;
    for source in [
        "int defined(int x) { return x; }\nint f(void) { return defined(1); }\n",
        "#define CALL defined(1)\n",
        "#define FAKE #if defined(1)\n",
        "int f(void) { use #if defined(1); }\n",
        "int f(void) {\n#pragma defined(1)\n}\n",
        "int f(void) {\n#pragma retain #if defined(1)\n}\n",
        "int f(void) {\n#if Defined(1) && defined_helper(2)\n#endif\n}\n",
        "int f(void) {\n/* #if defined(1) */\nuse(\"#if defined(1)\");\n}\n",
    ] {
        assert_eq!(
            scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
            scan_with(source.as_bytes(), Mode::C, without),
            "ordinary reading {source:?}"
        );
    }
}

#[test]
fn atomic_types_preserve_unsupported_operands_and_macro_readings() {
    let mut without = Corrections::ENABLED;
    without.atomic_types = false;
    for source in [
        "_Atomic int qualifier_object;".to_string(),
        "_Atomic_Helper(int) opaque_object;".to_string(),
        "_Atomic() empty_object;".to_string(),
        "_Atomic(TYPEOF(int)) macro_type_object;".to_string(),
        "_Atomic(int (*)(void)) nested_type_object;".to_string(),
        "_Atomic(int; int after_unfinished;".to_string(),
        "_Atomic(/* unclosed comment int) object;".to_string(),
        "_Atomic(/* spliced \\\n*/ int) object;".to_string(),
        "_Atomic(\n#if CHOICE\nint\n#endif\n) object;".to_string(),
        "#define TYPE _Atomic(int)\nint after_macro;".to_string(),
        format!("_Atomic({}int) object;", " ".repeat(256)),
        format!("_Atomic(/*{}*/int) object;", " ".repeat(256)),
    ] {
        assert_eq!(
            scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
            scan_with(source.as_bytes(), Mode::C, without),
            "unsupported atomic type {source:?}"
        );
    }
}

#[test]
fn attribute_comment_gaps_stay_within_the_declarator_window() {
    let source = |gap: &str| {
        format!(
            "int __attribute__{gap}((aligned(aligner()))) actual(int x) {{ target(); return x; }}\n"
        )
    };
    assert_eq!(
        scan_with(
            source(&" ".repeat(256)).as_bytes(),
            Mode::C,
            Corrections::ENABLED
        ),
        scan_with(source(" ").as_bytes(), Mode::C, Corrections::ENABLED),
        "the existing whitespace-only edge stays recognised"
    );
    let mut without_attributes = Corrections::ENABLED;
    without_attributes.attribute_expressions = false;
    for gap in [
        "/* unfinished".to_string(),
        format!("/*{}*/", "x".repeat(256)),
        "/* continued \\\n*/".to_string(),
        "/* continued \\\r\n*/".to_string(),
        "#if ENABLE\n#endif\n".to_string(),
        "// next line\n".to_string(),
    ] {
        let source = format!(
            "int f(void) __attribute__ {gap} ((annotate(helper()))) {{ target(); return 0; }}\n"
        );
        let with = scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED);
        let without = scan_with(source.as_bytes(), Mode::C, without_attributes);
        assert_eq!(with, without, "unsupported attribute gap {gap:?}");
    }
}

#[test]
fn typedef_decorations_preserve_unsupported_prefixes_and_suffixes() {
    let mut without = Corrections::ENABLED;
    without.typedef_decorations = false;
    for source in [
        "typedef int * API alias;",
        "typedef CONST sds(alias);",
        "typedef unsigned sds(alias);",
        "typedef RET_TYPE(int) actual ATTR;",
        "typedef int actual __attribute__((unused));",
        "typedef int actual __declspec(align(8));",
        "typedef int API incomplete(unknown);",
        "typedef int API nested((alias))(void);",
        "typedef int API unfinished[",
        "typedef int 1 API alias;",
        "typedef int \"text\" API alias;",
        "typedef int + API alias;",
    ] {
        assert_eq!(
            scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
            scan_with(source.as_bytes(), Mode::C, without),
            "unsupported typedef {source:?}"
        );
    }
    for suffix in [
        format!("{}alias;", " ".repeat(256)),
        format!("{};", "a".repeat(256)),
        "/* unfinished".to_string(),
        "/* continued \\\n*/ alias;".to_string(),
        "#if ENABLE\nalias;\n#endif".to_string(),
    ] {
        let source = format!("typedef int API {suffix}");
        assert_eq!(
            scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED),
            scan_with(source.as_bytes(), Mode::C, without),
            "unsupported typedef suffix {suffix:?}"
        );
    }
}

#[test]
fn typedef_decorations_restore_each_arm_and_join_conservatively() {
    let source = b"typedef int\n#ifdef FIRST\n*\n#else\nAPI in_other_arm,\n#endif\nAFTER_JOIN joined;\n\
                   typedef int\n#ifdef FIRST\nRET_TYPE(int)\n#else\nAPI in_word_arm,\n#endif\nAFTER_GROUP group_joined;\n\
                   typedef int\n#define NOISE * (unclosed\nAPI after_macro;\n";
    let mut marks = Vec::new();
    for record in scan_with(source, Mode::C, Corrections::ENABLED) {
        for symbol in &record.symbols {
            marks.push((
                record.line,
                symbol.mark.byte(),
                record.text[symbol.at..symbol.at + symbol.len].to_vec(),
            ));
        }
    }
    for line in [5, 12, 17] {
        assert!(marks.contains(&(line, b' ', b"API".to_vec())), "{marks:?}");
    }
    for name in [b"AFTER_JOIN".as_slice(), b"AFTER_GROUP"] {
        assert!(
            marks
                .iter()
                .any(|(_, mark, word)| *mark == b't' && word == name),
            "{marks:?}"
        );
    }
}

/// Writes `tests/fixtures/scanner/intended/` from the enabled reading, one fixture per
/// probe whose records differ from the reference's, and removes the others, naming each
/// file it writes or removes. It refuses while the reference reading differs from any
/// probe's cscope records, so a baseline bug is never saved as an intended reading. Run
/// it after an intended change of the enabled reading (ADR 0002), then review the diff:
/// `cargo test --bin ripscope write_intended_fixtures -- --ignored --nocapture`.
#[test]
#[ignore = "writes fixtures"]
fn write_intended_fixtures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner");
    let latin1 = |bytes: &[u8]| -> String { bytes.iter().map(|byte| char::from(*byte)).collect() };
    let mut entries: Vec<_> = std::fs::read_dir(root.join("probes"))
        .expect("probe directory")
        .map(|entry| entry.expect("probe entry").path())
        .collect();
    entries.sort();
    let baseline: Vec<String> = entries
        .iter()
        .flat_map(|probe| {
            probe_differences(
                probe,
                &fixture_path(probe, "expected"),
                Corrections::default(),
            )
        })
        .collect();
    assert!(
        baseline.is_empty(),
        "the reference reading differs from cscope; fix that first:\n{}",
        baseline.join("\n")
    );
    std::fs::create_dir_all(root.join("intended")).expect("intended directory");
    for probe in &entries {
        let wanted = fixture_path(probe, "intended");
        let reference = fixture_path(probe, "expected");
        if probe_differences(probe, &reference, Corrections::ENABLED).is_empty() {
            if std::fs::remove_file(&wanted).is_ok() {
                println!("removed {}", wanted.display());
            }
            continue;
        }
        println!("wrote {}", wanted.display());
        let name = probe
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let bytes = std::fs::read(probe).expect("probe");
        let records: Vec<Value> = scan_with(
            &bytes,
            Mode::for_name(name.as_bytes()),
            Corrections::ENABLED,
        )
        .iter()
        .map(|record| {
            let shape = shape_of(record);
            serde_json::json!({
                "line": shape.line,
                "fragments": shape.fragments.iter().map(|f| latin1(f)).collect::<Vec<_>>(),
                "symbols": shape
                    .symbols
                    .iter()
                    .map(|(mark, name, assigned)| {
                        serde_json::json!([latin1(&[*mark]), latin1(name), assigned])
                    })
                    .collect::<Vec<_>>(),
                "define_end": shape.define_end,
            })
        })
        .collect();
        let fixture = serde_json::json!({
            "probe": format!("probes/{name}"),
            "note": "The enabled reading, where it differs from the reference's records in \
                     ../expected/.",
            "records": records,
        });
        let mut text = serde_json::to_string_pretty(&fixture).expect("json");
        text.push('\n');
        std::fs::write(&wanted, text).expect("write intended fixture");
    }
}

/// With the enabled Corrections every probe reads as its intended output
/// (`tests/fixtures/scanner/intended/`), which exists only where the enabled reading
/// differs from the reference's, and as the reference's elsewhere.
#[test]
fn enabled_reading_matches_every_intended_or_reference_probe() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner/probes");
    let mut entries: Vec<_> = std::fs::read_dir(probes)
        .expect("probe directory")
        .map(|entry| entry.expect("probe entry").path())
        .collect();
    entries.sort();
    let mut failures = Vec::new();
    let mut intended = 0;
    for probe in &entries {
        let reference = fixture_path(probe, "expected");
        let wanted = fixture_path(probe, "intended");
        if wanted.exists() {
            intended += 1;
            assert!(
                !probe_differences(probe, &reference, Corrections::ENABLED).is_empty(),
                "{}: the enabled reading now equals the reference's. Either a Correction \
                 regressed, or the reading changed on purpose (ADR 0002) and the fixtures \
                 need rewriting with write_intended_fixtures",
                wanted.display()
            );
            failures.extend(probe_differences(probe, &wanted, Corrections::ENABLED));
        } else {
            failures.extend(probe_differences(probe, &reference, Corrections::ENABLED));
        }
    }
    assert!(intended > 0, "no intended outputs found");
    // An intended fixture without its probe would never be checked.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scanner");
    for fixture in std::fs::read_dir(root.join("intended")).expect("intended directory") {
        let fixture = fixture.expect("fixture").path();
        let stem = fixture
            .file_stem()
            .expect("stem")
            .to_string_lossy()
            .into_owned();
        assert!(
            entries.iter().any(|probe| probe
                .file_stem()
                .is_some_and(|probe| probe == stem.as_str())),
            "{}: no probe",
            fixture.display()
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Each record as `line:` and its symbols' marks and names, for the Correction tests.
fn marks(source: &str, corrections: &str) -> String {
    let corrections = super::Corrections::parse(corrections).expect("corrections");
    super::scan_with(source.as_bytes(), Mode::C, corrections)
        .iter()
        .map(|record| {
            let symbols: Vec<String> = record
                .symbols
                .iter()
                .map(|symbol| {
                    let name = &record.text[symbol.at..symbol.at + symbol.len];
                    format!(
                        "{}{}",
                        char::from(symbol.mark.byte()),
                        String::from_utf8_lossy(name).trim()
                    )
                })
                .collect();
            format!("{}:{}", record.line, symbols.join(","))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn linkage_blocks_do_not_nest() {
    let source = "#ifdef __cplusplus\nextern \"C\" {\n#endif\nstruct ss { int a; };\n\
        enum ee { RED };\nint counter;\nstatic int twice(int x) { return helper(x); }\n\
        #ifdef __cplusplus\n}\n#endif\nint after;\n";
    assert_eq!(
        marks(source, ""),
        "1: __cplusplus 4:sss, a 5:eee, RED 6: counter 7: twice, x, helper \
         8: __cplusplus 11: after"
    );
    assert_eq!(
        marks(source, "linkage-blocks"),
        "1: __cplusplus 4:sss,ma 5:eee,mRED 6:gcounter 7:$twice, x,`helper,} \
         8: __cplusplus 11:gafter"
    );
    // An extern declaration with a linkage string is still an extern declaration.
    assert_eq!(
        marks("extern \"C\" int f(void);\nint g;\n", "linkage-blocks"),
        "1: f 2:gg"
    );
}

#[test]
fn definitions_have_bodies() {
    let source = "int kr(a, b)\n  int a;\n  char *b;\n{\n  use(a);\n}\n\
        void (*signal(int sig, void (*func)(int)))(int)\n{\n  other(sig);\n}\n\
        extern void *xmalloc (unsigned n) __attribute__((malloc));\nint g1;\n\
        XMLPARSEAPI(void)\nXML_Set(int p);\nint g2;\n\
        typedef voidpf (*alloc_func)(voidpf opaque);\nint g3;\n\
        PHP_FUNCTION(strlen)\n{\n  call(1);\n}\n\
        int f(void)\n#ifdef X\n{ a();\n#else\n{ b();\n#endif\n}\nint g4;\n";
    assert_eq!(
        marks(source, "definition-bodies"),
        "1:$kr, a, b 2: a 3: b 5:`use, a 6:} 7:$signal, sig, func 9:`other, sig 10:} \
         11: xmalloc, n, __attribute__, malloc 12:gg1 13: XMLPARSEAPI 14: XML_Set, p \
         15:gg2 16: voidpf,talloc_func,tvoidpf,topaque 17:gg3 18:$PHP_FUNCTION, strlen \
         20:`call 21:} 22:$f 23: X 24:`a 26:`b 28:} 29:gg4"
    );
    // A function returning a function pointer, and a body read as a function's where the
    // head's own parameter list is beyond the definition shape.
    assert_eq!(
        marks(
            "int (*get(const S *s))(const S *s, int op)\n{\n  return s->cb;\n}\nint g;\n",
            "definition-bodies"
        ),
        "1:$get, S, s, op 3: s, cb 4:} 5:gg"
    );
    assert_eq!(
        marks(
            "int twice(int (*outer)(int (*inner)(int)), int y) { return outer(0) + y; }\n",
            "definition-bodies"
        ),
        "1: twice, outer, inner, y,`outer,}"
    );
    // A wrapper macro before the head keeps the reference's reading: both are definitions,
    // and the body is the last one's.
    assert_eq!(
        marks(
            "  FT_LOCAL_DEF( FT_Error )\n  get( FT_Face  face )\n  {\n    return f( face );\n  }\n",
            "definition-bodies"
        ),
        "1:$FT_LOCAL_DEF, FT_Error 2:$get, FT_Face, face 4:`f, face 5:}"
    );
    // A head inside a definition's parameter list is a parameter's type.
    assert_eq!(
        marks(
            "int r(T (*f)(T, int), T x)\n{\n  f(x, 1);\n}\n",
            "definition-bodies"
        ),
        "1:$r, T, f, x 3:`f, x 4:}"
    );
    // Arms that each open a parenthesis leave the reference's count one too high; a later
    // head is still at the top level.
    assert_eq!(
        marks(
            "void f(void) {\n#ifdef X\n  warn(\n#else\n  err(\n#endif\n  \"m\");\n}\nstatic int g(int a)\n{\n  return a;\n}\n",
            "definition-bodies"
        ),
        "1:$f 2: X 3:`warn 5:`err 8:} 9:$g, a 11: a 12:}"
    );
    // A keyword before a parenthesised declarator is only a keyword.
    assert_eq!(
        marks(
            "typedef int (*handler)(int);\nint after;\nstruct later { int m; };\n",
            "definition-bodies"
        ),
        "1:thandler 2:gafter 3:slater,mm"
    );
    // The reference reads the declarations as definitions, and what follows as inside
    // them.
    assert_eq!(
        marks(
            "extern void *xmalloc (unsigned n) __attribute__((malloc));\nint g1;\n",
            ""
        ),
        "1:$xmalloc, n,`__attribute__, malloc 2: g1"
    );
}

#[test]
fn declarations_that_only_look_like_definitions_declare_nothing_else() {
    // A macro head with one plain word is not a K&R head when the `{` that follows is
    // an aggregate's, or does not come right after the declaration's `;`.
    assert_eq!(
        marks(
            "typedef STACK_OF(X) Y;\nstruct z {\n  int m;\n};\nint g;\n",
            "definition-bodies"
        ),
        "1: STACK_OF,tX,tY 2:sz 3:mm 5:gg"
    );
    assert_eq!(
        marks(
            "MACRO()\nstruct x {\n  int m;\n};\nint g;\n",
            "definition-bodies"
        ),
        "1: MACRO 2:sx 3:mm 5:gg"
    );
    // A head on a directive's continuation line never opens a function.
    assert_eq!(
        marks(
            "#if defined(A) \\\n  && !defined(B)\n#endif\nint g;\n",
            "definition-bodies"
        ),
        "1: defined, A 2: defined, B 4:gg"
    );
    // Words after a function declaration's parameter list declare nothing.
    assert_eq!(
        marks("char *f(int n) FAST_FUNC;\nint g;\n", "definition-bodies"),
        "1: f, n, FAST_FUNC 2:gg"
    );
    // A directive's continuation lines define nothing.
    let continued = "#if defined(A) || \\\n  (X < 5)\n#endif\nint g;\n";
    assert_eq!(marks(continued, ""), "1: defined, A 2:gX 4:gg");
    assert_eq!(
        marks(continued, "directive-continuations"),
        "1: defined, A 2: X 4:gg"
    );
    // A linkage block may open on a later line.
    assert_eq!(
        marks("extern \"C\"\n{\nint a;\n}\n", "linkage-blocks"),
        "3:ga"
    );
}

#[test]
fn a_backslash_before_crlf_continues_the_line() {
    let macro_line = "#define X(a) \\\r\n  do { a; } while (0)\r\nint g;\r\n";
    assert_eq!(marks(macro_line, ""), "1:#X, a 2:ga 3: g");
    assert_eq!(marks(macro_line, "crlf-continuations"), "1:#X, a 2: a 3:gg");
    let literal = "char *s = \"a \\\r\nb\";\r\nint g;\r\n";
    assert_eq!(marks(literal, ""), "1:gs 2: b 3: g");
    assert_eq!(marks(literal, "crlf-continuations"), "1:gs 3:gg");
}

#[test]
fn unspaced_conditionals_are_conditionals() {
    let source =
        "int f(void) {\n#if(A)\n  {\n#elif!defined(B)\n  {\n#endif\n  g();\n  }\n}\nint h;\n";
    assert_eq!(marks(source, ""), "1:$f 2: A 4:`defined, B 7:`g 10: h");
    assert_eq!(
        marks(source, "unspaced-conditionals"),
        "1:$f 2: A 4:`defined, B 7:`g 9:} 10:gh"
    );
}

fn assert_conditional_reading(source: &str, expected: &str) {
    for corrections in [Corrections::default(), Corrections::ENABLED] {
        for source in [source.to_owned(), source.replace('\n', "\r\n")] {
            let actual = scan_with(source.as_bytes(), Mode::C, corrections)
                .iter()
                .map(|record| show(&shape_of(record)))
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(actual, expected, "{}: {source:?}", corrections.names());
        }
    }
}

#[test]
fn nested_conditionals_keep_the_last_directives_else_state() {
    // Opening the inner group clears the global else state, even though the outer
    // group has an else. Its following #if therefore continues the outer group.
    let reset = "#if OUTER\nvoid first(void) {\n#else\n#if INNER\nvoid second(void) {\n\
        #endif\n#endif\n#if NEXT\nvoid third(void) {\n#endif\ntarget();\n}\nint after;\n";
    // An inner else instead prevents continuation of the outer group, which has
    // no else of its own. Keep this permissive reading, including its open depth.
    let inner_else = "#if OUTER\n#if INNER\nvoid first(void) {\n#else\nvoid second(void) {\n\
        #endif\n#endif\n#if NEXT\nvoid third(void) {\n#endif\ntarget();\n}\nint after;\n";
    for (source, expected) in [
        (
            reset,
            "1 #if [ OUTER]\n2 void [$first](void ) {\n4 #if [ INNER]\n\
             5 void [$second](void ) {\n8 #if [ NEXT]\n9 void [$third](void ) {\n\
             11 [`target]();\n12 [}]}\n13 int [gafter];",
        ),
        (
            inner_else,
            "1 #if [ OUTER]\n2 #if [ INNER]\n3 void [$first](void ) {\n\
             5 void [$second](void ) {\n8 #if [ NEXT]\n9 void [`third](void ) {\n\
             11 [`target]();\n13 int [ after];",
        ),
    ] {
        assert_conditional_reading(source, expected);
    }
}

#[test]
fn unmatched_conditionals_and_elif_keep_permissive_continuation() {
    let source = "#else\n#elif LOST\n#endif\n#if NEXT\nvoid first(void) {\n\
        #elif OTHER\nvoid second(void) {\n#endif\n#if LAST\nvoid third(void) {\n\
        #endif\ntarget();\n}\nint after;\n#endif\n#if(EXTRA)\nint tail;\n#endif\n";
    let expected = "2 #elif [ LOST]\n4 #if [ NEXT]\n5 void [$first](void ) {\n\
        6 #elif [ OTHER]\n7 void [$second](void ) {\n9 #if [ LAST]\n\
        10 void [$third](void ) {\n12 [`target]();\n13 [}]}\n14 int [gafter];\n\
        16 #if([ EXTRA])\n17 int [gtail];";
    assert_conditional_reading(source, expected);
}

#[test]
fn a_macro_definition_changes_nothing_after_it() {
    let typedef = "#define T(c) typedef int t[(c)?1:-1]\n#define N 1\nint g;\n";
    assert_eq!(marks(typedef, ""), "1:#T, c,tt 2:tN 3:tg");
    assert_eq!(marks(typedef, "macro-bodies"), "1:#T, c,tt 2:#N 3:gg");
    let brace = "#define BEGIN {\nint f(void) { return h(); }\nint g;\n";
    assert_eq!(marks(brace, ""), "1:#BEGIN 2: f, h 3:gg");
    assert_eq!(marks(brace, "macro-bodies"), "1:#BEGIN 2:$f,`h,} 3:gg");
    // A `//` comment at the end of a `#define` line continues the macro in the reference,
    // so the next `#define` is read inside it.
    let chained = "struct s {\n  int a;\n#define X 1 // c\n#define Y 2\n  int b;\n};\nint g;\n";
    assert_eq!(
        marks(chained, "macro-bodies"),
        "1:ss 2:ma 3:#X 4:#Y 5:mb 7:gg"
    );
    let inside = "int f(void) {\n#define END }\n  h();\n}\nint g;\n";
    assert_eq!(marks(inside, ""), "1:$f 2:#END,},) 3: h 5:gg");
    assert_eq!(marks(inside, "macro-bodies"), "1:$f 2:#END 3:`h 4:} 5:gg");
}

#[test]
fn round_two_readings() {
    // A keyword head without a body declares nothing more (Lua's parenthesised names).
    let lua = "LUALIB_API int (luaL_x) (lua_State *L, int idx);\nint g;\n";
    assert_eq!(
        marks(lua, "definition-bodies"),
        "1: LUALIB_API, luaL_x, lua_State, L, idx 2:gg"
    );
    // A typedef of a function pointer keeps its name.
    assert_eq!(
        marks("typedef void (*cb)(int n);\nint g;\n", "definition-bodies"),
        "1:tcb,tn 2:gg"
    );
    // An invocation without `;` does not swallow the aggregate after it.
    assert_eq!(
        marks(
            "DECLARE_THING(a)\nstruct s {\n  size_t m;\n};\n",
            "definition-bodies"
        ),
        "1: DECLARE_THING, a 2:ss 3: size_t,mm"
    );
    // Designated initializer entries define nothing.
    let designated = "const struct ops o = {\n\t.a = x,\n\t.b = y,\n};\nint after;\n";
    assert_eq!(marks(designated, ""), "1: ops,go 2: a, x 3:gb, y 5:gafter");
    assert_eq!(
        marks(designated, "initializer-entries"),
        "1: ops,go 2: a, x 3: b, y 5:gafter"
    );
}

#[test]
fn parenthesised_declarators_declare_no_parameters() {
    // The names before the parameter lists keep the reference's reading (a follow-up).
    let source = "LUALIB_API const char *(luaL_tolstring) (lua_State *L, int idx);\n\
        static PyObject* (*py3_get)(PyObject *, Py_ssize_t n);\n\
        struct s { void (* cb)(struct h *hh); };\n";
    assert_eq!(
        marks(source, "definition-bodies"),
        "1: LUALIB_API,gluaL_tolstring, lua_State, L, idx \
         2:gPyObject,gpy3_get, PyObject, Py_ssize_t, n 3:ss,mcb, h, hh"
    );
}

#[test]
fn written_function_after_macro_heads_owns_its_body() {
    let source = "DECLARE(X, T)\nDECLARE(Y, U)\n\
        int real(T *x, STACK_OF(Y) *p) {\n    use_it(x);\n}\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:$DECLARE, X, T 2:$DECLARE, Y, U \
         3:$real, T, x, STACK_OF, Y, p 4:`use_it, x 5:}"
    );
    let (_, cost) = super::scan_with_cost(source.as_bytes(), Mode::C, Corrections::ENABLED);
    assert_eq!(cost.narrow_windows, 0);
}

#[test]
fn numeric_annotations_before_written_types_are_occurrences() {
    assert_eq!(
        marks(
            "static ANN(1) int f(void) { g(); }\nint after;\n",
            "enabled"
        ),
        "1: ANN,$f,`g,} 2:gafter"
    );
    assert_eq!(
        marks(
            "static PRINTF_LIKE(2, 3) void logprintf(Context *ctx, const char *fmt, ...)\n\
             { emit(ctx); }\n",
            "enabled"
        ),
        "1: PRINTF_LIKE,$logprintf, Context, ctx, fmt 2:`emit, ctx,}"
    );
}

#[test]
fn annotation_prefixes_preserve_real_and_ambiguous_heads() {
    for source in [
        "int kept(int value) { use_it(value); }\n",
        "kept(value) int value; { use_it(value); }\n",
        "kept(1) { use_it(); }\n",
        "kept() { use_it(); }\n",
        "kept(value) int later(void) { use_it(); }\n",
        "kept(1) Result later(void) { use_it(); }\n",
        "kept(1)\n#if ENABLED\nint later(void) { use_it(); }\n#endif\n",
    ] {
        assert!(
            marks(source, "enabled").contains("$kept"),
            "lost the head in {source}"
        );
    }
}

#[test]
fn pointer_return_types_are_occurrences_not_object_definitions() {
    let source = "static PyObject *(*get)(Other *o);\n\
        struct ops { PyObject *(*member)(void); };\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1: PyObject,gget, Other, o 2:sops, PyObject,mmember 3:gafter"
    );
}

#[test]
fn opaque_tails_preserve_written_declarations_and_enum_members() {
    let controls = "typedef int T;\nT\n#if A\nvalue\n#else\nother\n#endif\n;\n\
        struct S {\nT\nsplit_member\n};\n\
        struct Outer {\nenum Inner\n#if A\n{\n#else\n{\n#endif\nINNER_ENUM\n} member;\n};\n\
        #if A\nstruct Conditional {\n#else\nenum Conditional {\n#endif\nKEPT\n};\n";
    let without = Corrections::ENABLED
        .names()
        .replace(",bare-directive-words", "")
        .replace(",opaque-member-tails", "");
    // The type use before #if is also corrected; the declared names must survive.
    assert_eq!(
        marks(controls, "enabled"),
        marks(controls, &without).replace("2:gT", "2: T")
    );
    for name in ["gvalue", "gother", "msplit_member", "mINNER_ENUM", "mKEPT"] {
        assert!(marks(controls, "enabled").contains(name), "missing {name}");
    }
}

#[test]
fn bare_words_before_directives_are_occurrences() {
    for (source, expected) in [
        (
            "#ifndef CORE\nINIT\n#else\nint core;\n#endif\n",
            "1: CORE 2: INIT 4:gcore",
        ),
        (
            "#if A\nDWORD\n#else\nUINT\n#endif\nf(void) {}\n",
            "1: A 2: DWORD 4: UINT 6:$f,}",
        ),
        ("#if A\nint after;\nEND\n#endif\n", "1: A 2:gafter 3: END"),
    ] {
        assert_eq!(marks(source, "enabled"), expected);
    }
}

#[test]
fn opaque_tails_preserve_c_and_implementation_declaration_words() {
    let without = Corrections::ENABLED
        .names()
        .replace(",bare-directive-words", "")
        .replace(",opaque-member-tails", "");
    for specifier in [
        "_Atomic",
        "_Thread_local",
        "__thread",
        "__extension__",
        "restrict",
        "thread_local",
        "_Alignas(16)",
        "__attribute__((aligned(16)))",
        "__declspec(align(16))",
        "typeof(int)",
        "_Atomic(T)",
    ] {
        let source = format!(
            "typedef int T;\n#if A\n{specifier} T global\n#endif\n;\n\
             struct S {{\n{specifier} T member\n}};\n"
        );
        assert_eq!(
            marks(&source, "enabled"),
            marks(&source, &without),
            "{specifier}"
        );
    }
    for source in [
        "EXTERN T global\n#if A\nINIT(0);\n#else\nINIT(1);\n#endif\n",
        "struct S {\nTYPEOF(T) member\n};\n",
        "struct S {\nALIGN(16) T member\n};\n",
    ] {
        assert_eq!(marks(source, "enabled"), marks(source, &without));
    }
}

#[test]
fn a_provisional_head_does_not_lend_its_type_to_a_trailing_annotation() {
    let source = "FORCE_INLINE()\nstatic int\nf(int a, WRAP(int b))\n\
        CHECK(condition < 0, \"error\")\nDECOR CHECK(used(x))\n{ target(); }\n";
    let records = super::scan_with(source.as_bytes(), Mode::C, Corrections::ENABLED);
    assert!(
        records
            .iter()
            .flat_map(|r| r.symbols.iter().map(move |s| (r, s)))
            .all(|(r, s)| s.mark != crate::unit::Mark::Function
                || &r.text[s.at..s.at + s.len] != b"CHECK")
    );
}

#[test]
fn heads_the_definition_shape_rejects_are_definitions_when_a_body_follows() {
    let stack = "int f(S *s, STACK_OF(X) *sk)\n{\n  int i;\n  g(i);\n}\nint after;\n";
    assert_eq!(
        marks(stack, ""),
        "1: f, S, s, STACK_OF, X, sk 3:gi 4: g, i 6:gafter"
    );
    assert_eq!(
        marks(stack, "definition-heads"),
        "1:$f, S, s, STACK_OF, X, sk 3: i 4:`g, i 5:} 6:gafter"
    );
    // An attribute before a head is not one, nor is an annotation after one.
    assert_eq!(
        marks(
            "__attribute__((constructor)) static void init(void) { g(); }\n",
            "definition-heads"
        ),
        "1: __attribute__, constructor,$init,`g,}"
    );
    let annotated = "static void f(struct r *r)\n  __releases(r->lock)\n{\n  g(r);\n}\n";
    assert_eq!(marks(annotated, "definition-heads"), marks(annotated, ""));
    // Alone it leaves the reference's reading of the other heads, and the calls after a
    // declaration the reference takes for a definition, as they are.
    let declared = "int f(int) ATTRIBUTE;\nint g(STACK_OF(X) *sk)\n{\n  h();\n}\n";
    assert_eq!(marks(declared, "definition-heads"), marks(declared, ""));
    assert_eq!(
        marks(declared, "definition-bodies,definition-heads"),
        "1: f, ATTRIBUTE 2:$g, STACK_OF, X, sk 4:`h 5:}"
    );
}

#[test]
fn written_function_heads_own_callback_parameters_and_trailing_annotations() {
    assert_eq!(
        marks("int f(int callback(int)) { return target(); }\n", "enabled"),
        "1:$f, callback,`target,}"
    );
    assert_eq!(
        marks(
            "static inline void lock(struct lock *p) __acquires(p)\n{ target(p); }\n",
            "enabled"
        ),
        "1: inline,$lock, lock, p, __acquires 2:`target, p,}"
    );
    assert_eq!(
        marks(
            "extern API(result_t)\nget(void)\n{ return target(); }\n",
            "enabled"
        ),
        "1:$API, result_t 2:$get 3:`target,}"
    );
    assert_eq!(
        marks(
            "static void printflike(3,4)\nlog(int x) { target(); }\n",
            "enabled"
        ),
        "1:$printflike 2:$log, x,`target,}"
    );
    assert_eq!(
        marks(
            "void\nlock(void) __acquires(resource)\n{ target(); }\n",
            "enabled"
        ),
        "2:$lock, __acquires, resource 3:`target,}"
    );
    assert_eq!(
        marks("static WRAP(int f(int x)) { target(); }\n", "enabled"),
        "1: WRAP,$f, x,`target,}"
    );
    assert_eq!(
        marks(
            "#if FLAG\nint\nf(int x)\n#else\nint\ng(int x)\n#endif\n{ target(); }\n",
            "enabled"
        ),
        "1: FLAG 3:$f, x 6:$g, x 8:`target,}"
    );
    assert_eq!(
        marks(
            "int\n#if FLAG\nf(void)\n#else\ng(void)\n#endif\n{ target(); }\n",
            "enabled"
        ),
        "2: FLAG 3:$f 5:$g 7:`target,}"
    );
    assert_eq!(
        marks(
            "int f(void)\n#if FLAG\n__acquires(first)\n#else\n__acquires(second)\n#endif\n{ target(); }\n",
            "enabled"
        ),
        "1:$f 2: FLAG 3: __acquires, first 5: __acquires, second 7:`target,}"
    );
}

#[test]
fn array_bounds_in_a_function_head_keep_expression_calls() {
    assert_eq!(
        marks(
            "void f(int n, int values[bound(n)]) { target(); }\n",
            "enabled"
        ),
        "1:$f, n, values,`bound,`target,}"
    );
    // A callback declaration is still a type, not a call.
    assert_eq!(
        marks(
            "void f(int callback(int), int values[bound()]) { target(); }\n",
            "enabled"
        ),
        "1:$f, callback, values,`bound,`target,}"
    );
    assert_eq!(
        marks(
            "void f(int values[table[index()] + bound()]) { target(); }\n",
            "enabled"
        ),
        "1:$f, values, table,`index,`bound,`target,}"
    );
}

#[test]
fn attribute_labels_are_names_and_their_expression_payloads_keep_calls() {
    let source = "__attribute__((annotate(payload()))) int f(void) {\nint __attribute__((annotate(helper()))) value;\n__declspec(align(bound())) int other;\n}\n";
    assert_eq!(
        marks(source, "enabled"),
        "1: __attribute__, annotate, payload,$f 2: __attribute__, annotate,`helper, value 3: __declspec, align,`bound, other 4:}"
    );
    assert_eq!(
        marks(
            "typedef struct __attribute__((aligned(64))) state { unsigned long value; } state;\n",
            "enabled"
        ),
        "1: __attribute__, aligned,sstate,mvalue,tstate"
    );
    assert_eq!(
        marks(
            "typedef void (__attribute__((nocf_check)) *trigger_t)(void);\n",
            "enabled"
        ),
        "1: __attribute__, nocf_check,ttrigger_t"
    );
}

#[test]
fn lexical_character_classes_do_not_open_c_array_bounds() {
    let source = b"%%\n[^[] return TOKEN;\n%%\nstruct S { int member; };\n";
    let records = super::scan_with(source, Mode::Lex, Corrections::ENABLED);
    assert!(records.iter().any(|record| {
        record.symbols.iter().any(|symbol| {
            symbol.mark == crate::unit::Mark::Member
                && &record.text[symbol.at..symbol.at + symbol.len] == b"member"
        })
    }));
}

#[test]
fn closed_conditional_tail_comments_cannot_create_facts() {
    let source = "#if FLAG\nint inside;\n#endif /* comment\nint phantom;\n#include \"ghost.h\"\n*/\nint after;\n";
    assert_eq!(marks(source, "enabled"), "1: FLAG 2:ginside 7:gafter");
    // Code on the closing line is outside this Correction's closed-tail profile.
    let trailing = "#if FLAG\n#endif /* comment\n*/ int after;\n";
    assert_eq!(marks(trailing, "enabled"), marks(trailing, ""));
    // A splice can form a closing delimiter. It is outside the simple closed-tail
    // proof, which must never swallow a real declaration after that delimiter.
    let spliced = "#if FLAG\n#endif /* comment *\\\n/\nint real;\n/* next */\nint after;\n";
    for source in [spliced.to_owned(), spliced.replace('\n', "\r\n")] {
        assert_eq!(marks(&source, "enabled"), marks(&source, ""));
    }
    let without = Corrections::ENABLED
        .names()
        .replace(",conditional-comments", "");
    for tail in [
        "unclosed\nint phantom;".to_owned(),
        format!("{}\n*/", "a".repeat(65536)),
    ] {
        let source = format!("#if FLAG\n#endif /* {tail}\nint after;\n");
        assert_eq!(marks(&source, "enabled"), marks(&source, &without));
    }
}

#[test]
fn macro_names_can_have_keyword_spellings() {
    let source =
        "#define const\n#define inline __inline__\n#define struct(x) replacement(x)\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:#const 2:#inline, __inline__ 3:#struct, x,`replacement 4:gafter"
    );
    let without = Corrections::ENABLED.names().replace(",keyword-macros", "");
    for keyword in super::keywords::KEYWORDS
        .iter()
        .filter(|word| !word.word.starts_with(b"#"))
    {
        let source = format!(
            "#define {}(arg) replacement(arg)\n",
            String::from_utf8_lossy(keyword.word)
        );
        let records = |switches: &str| {
            super::scan_with(
                source.as_bytes(),
                Mode::C,
                Corrections::parse(switches).unwrap(),
            )
        };
        let before = records(&without);
        let after = records("enabled");
        assert_eq!(before[0].text, after[0].text, "{source}");
    }
}

#[test]
fn round_three_readings() {
    // A list that wraps a whole head is the wrapper's: the reference's inner head stays.
    let wrapped = "UV_UNUSED(static int uv__stat(const char* path, struct stat* s)) {\n\
        \x20 return stat(path, s);\n}\n\
        int __NTH (tolower (int __c))\n{\n  return __c;\n}\n\
        static int pyf(PyObject *self, void *Py_UNUSED(ignored)) { return g(self); }\n\
        static VALUE grep_i(RB_BLOCK_CALL_FUNC_ARGLIST(i, args)) { return h(i); }\n";
    assert_eq!(
        marks(wrapped, "all"),
        "1: UV_UNUSED,$uv__stat, path, stat, s 2:`stat, path, s 3:} 4: __NTH,$tolower, __c 6: __c 7:} 8:$pyf, PyObject, self, Py_UNUSED, ignored,`g,} 9: VALUE,$grep_i, RB_BLOCK_CALL_FUNC_ARGLIST, i, args,`h,}"
    );
    // A call on an `#if` line in a parameter list is one from the function.
    let conditional = "void f(int a\n#if CHECK(D)\n  , int b\n#endif\n  )\n{ g(a); }\nint after;\n";
    assert_eq!(
        marks(conditional, "all"),
        "1:$f, a 2:`CHECK, D 3: b 6:`g, a,} 7:gafter"
    );
    // A macro's arguments before a declarator group are no declarator group, and a
    // declarator list ends at the next `;`, `{` or `}` even with a parenthesis left open.
    let declarators = "PyAPI_DATA(int) (*PyOS_InputHook)(void);\n\
        struct ops {\n  STACK_OF(X509) (*get)(void *ctx);\n\
        #ifdef X\n  int (*open)(struct file *f\n#else\n  int (*open)(struct file *f, int flags\n#endif\n  );\n\
        \x20 int count;\n};\nint after;\n";
    assert_eq!(
        marks(declarators, "all"),
        "1: PyAPI_DATA,gPyOS_InputHook 2:sops 3: STACK_OF, X509,mget, ctx 4: X 5:mopen, file, f 7: open, file, f, flags 10:mcount 12:gafter"
    );
    // An attribute, or a name in a tag's place, is never a head.
    let attributes = "struct __attribute__((packed)) { int a; } s1;\n\
        enum __attribute__((packed)) { EA, EB } e1;\nint after;\n";
    assert_eq!(
        marks(attributes, "all"),
        "1: __attribute__, packed,ma,gs1 2: __attribute__, packed,mEA,mEB,ge1 3:gafter"
    );
    let tagged = "union PACKED(F(1)) { int d; } u1;\nint after;\n";
    assert_eq!(marks(tagged, "definition-heads"), marks(tagged, ""));
    assert_eq!(marks(tagged, "all"), "1: PACKED,$F, d,},gu1 2:gafter");
    // Declarator lists: after a name that starts its line, after a macro split over
    // lines, in a member after `void`, after a group split over lines, and one holding a
    // union, whose `;` and `}` are its own.
    let lists = "unsigned char\n(stdc_bit_ceil_uc) (unsigned char x);\n\
        LZMA_SYMVER_API(\"lzma_block@XZ_5.0\",\n  lzma_ret, name)(lzma_block *block, int n);\n\
        struct file_ops {\n  void (release)(struct file *f);\n  int (*\n   open)(struct file *g, int flags);\n\
        \x20 long (*fm)(int, union { long *x; const long *y; } u, long z);\n  int count;\n};\n\
        int after;\n";
    assert_eq!(
        marks(lists, "definition-bodies"),
        "2:gstdc_bit_ceil_uc, x 3: LZMA_SYMVER_API 4: lzma_ret, name, lzma_block, block, n 5:sfile_ops 6:mrelease, file, f 8:mopen, file, g, flags 9:mfm, x, y, u, z 10:mcount 12:gafter"
    );
}

#[test]
fn a_line_comment_ends_a_macro_with_its_line() {
    let source =
        "#define ONE 1 // one\nint after;\n#define TWO 2 // two\n#define THREE 3\nint later;\n";
    assert_eq!(
        marks(source, ""),
        "1:#ONE 2: after 3:#TWO 4:#THREE 5:glater"
    );
    assert_eq!(
        marks(source, "macro-line-comments"),
        "1:#ONE 2:gafter 3:#TWO 4:#THREE 5:glater"
    );
    // The displayed line is the one without the comment: the blank before it is trailing.
    let view = |source: &str| {
        let corrections = super::Corrections::parse("macro-line-comments").expect("known");
        reference_view(&super::scan_with(source.as_bytes(), Mode::C, corrections)[0].text)
    };
    assert_eq!(view("#define ONE (1) // one\n"), view("#define ONE (1)\n"));
    // A backslash continues the comment onto the next line, and the macro with it.
    let continued = "#define ONE 1 // one \\\nint inside;\nint after;\n";
    assert_eq!(
        marks(continued, "macro-line-comments"),
        marks(continued, "")
    );
}

/// The lines whose records end a macro definition.
fn define_ends(source: &str, corrections: &str) -> Vec<u32> {
    let corrections = super::Corrections::parse(corrections).expect("corrections");
    super::scan_with(source.as_bytes(), Mode::C, corrections)
        .iter()
        .filter(|record| record.define_end_after)
        .map(|record| record.line)
        .collect()
}

#[test]
fn a_line_comment_ends_a_macro_where_c_ends_it() {
    let source = "#define ONE 1 // one\nint after;\n";
    assert_eq!(define_ends(source, ""), [2]);
    assert_eq!(define_ends(source, "macro-line-comments"), [1]);
    // The run-on also swallows an `#else`.
    let arms = "#if A\n#define X 1 // c\n#else\nint alt;\n#endif\nint after;\n";
    assert_eq!(marks(arms, ""), "1: A 2:#X 3: 4:galt 6:gafter");
    assert_eq!(
        marks(arms, "macro-line-comments"),
        "1: A 2:#X 4:galt 6:gafter"
    );
    // CR LF line ends; a backslash before CR LF continues the comment, with or without
    // crlf-continuations.
    let crlf = "#define ONE 1 // one\r\nint after;\r\n#define TWO 2 // two \\\r\nint inside;\r\nint later;\r\n";
    assert_eq!(
        marks(crlf, "crlf-continuations,macro-line-comments"),
        "1:#ONE 2:gafter 3:#TWO 4: inside 5:glater"
    );
    assert_eq!(
        define_ends(crlf, "crlf-continuations,macro-line-comments"),
        define_ends(crlf, "macro-line-comments")
    );
    // With macro-bodies, a run-on macro used to take a function's `}` into its body.
    let closing =
        "void f(void) {\n  a();\n#define A 1 // a\n}\nint after;\nvoid h(void) { k(); }\n";
    assert_eq!(
        marks(closing, "macro-bodies"),
        "1:$f 2:`a 3:#A 4: 5: after 6:`h,`k"
    );
    assert_eq!(
        marks(closing, "macro-bodies,macro-line-comments"),
        "1:$f 2:`a 3:#A 4:} 5:gafter 6:$h,`k,}"
    );
    // With macro-bodies, a run-on macro used to read a struct's members in its fresh
    // state.
    let members = "#define A 1 // a\nstruct s {\n  int x;\n};\nint after;\n";
    assert_eq!(marks(members, "macro-bodies"), "1:#A 2:ss 3:gx 5:gafter");
    assert_eq!(
        marks(members, "macro-bodies,macro-line-comments"),
        "1:#A 2:ss 3:mx 5:gafter"
    );
    // The displayed line is the reference's display of the line without the line
    // comment, also after a block comment.
    let view = |source: &str, corrections: &str| {
        let corrections = super::Corrections::parse(corrections).expect("known");
        reference_view(&super::scan_with(source.as_bytes(), Mode::C, corrections)[0].text)
    };
    assert_eq!(
        view("#define A (1) /* x */ // y\n", "macro-line-comments"),
        view("#define A (1) /* x */\n", "")
    );
    // With directive-continuations, the directive the run-on used to hide stays one.
    let continued = "#ifndef X\n#define X 0 // c\n#endif\n#if (defined(A) && B) ||\\\n    (defined(C) && X(v))\n#define Y 1\n#endif\nint after;\n";
    assert_eq!(
        marks(continued, "directive-continuations,macro-line-comments"),
        "1: X 2:#X 4: defined, A, B 5: defined, C, X, v 6:#Y 8:gafter"
    );
}

#[test]
fn pointer_declarators_define_the_declared_name() {
    // A member or global of each recognised shape. The reference reads `T (` as a call in
    // an aggregate, defining the parameter `a` there, and at file scope as a function head.
    let source = "struct s {\nT (*m1)(int a);\nT (* const m2)(int);\nT (*m3[4])(int);\n\
        T (^m4)(int);\nT (*m5)(int (*p)(int));\n};\nT (*g1)(int);\nstatic T (*g2)(int) = 0;\n\
        extern T (*e1)(int);\nvoid f(T (*p1)(int));\nvoid body(void) { T (*l1)(int) = 0; }\n";
    assert_eq!(
        marks(source, "definition-bodies"),
        "1:ss 2: T, m1, a 3: T, m2 4: T, m3 5: T, m4 6: T, m5, p 8: T, g1 9: T, g2 \
         10: T, e1 11: f, T, p1 12:$body,`T, l1,}"
    );
    assert_eq!(
        marks(source, "definition-bodies,pointer-declarators"),
        "1:ss 2: T,mm1, a 3: T,mm2 4: T,mm3 5: T,mm4 6: T,mm5, p 8: T,gg1 9: T,gg2 \
         10: T, e1 11: f, T, p1 12:$body,`T, l1,}"
    );
    // Alone it acts almost only in aggregates, where the reference's parameter members
    // stay and a parameter of a member's function-pointer parameter becomes one too: it
    // is meant to be used with definition-bodies.
    assert_eq!(
        marks(source, "pointer-declarators"),
        "1:ss 2: T,mm1,ma 3: T,mm2 4: T,mm3 5: T,mm4 6: T,mm5,mp 8:$T, g1 9:$T, g2 \
         10:$T, e1 11:`f,$T, p1 12:$body,`T, l1,}"
    );
    // The GNU spellings of the qualifiers, plain names as the reference records them.
    assert_eq!(
        marks(
            "struct q {\nT (* __volatile__ v1)(int);\nT (* __volatile v2)(int);\n\
             T (* __const__ c1)(int);\n};\n",
            "definition-bodies,pointer-declarators"
        ),
        "1:sq 2: T, __volatile__,mv1 3: T, __volatile,mv2 4: T, __const__,mc1"
    );
    // A pointer to an array: the reference defines it only after a keyword type.
    let arrays = "struct a {\nT (*rows)[4];\nconst T (*grid[2])[3];\nint (*kw)[4];\n};\n\
        T (*g)[8];\nextern T (*e)[8];\nvoid f(T (*p)[2]);\nstatic const T (*s)[4] = 0;\n";
    assert_eq!(
        marks(arrays, "definition-bodies"),
        "1:sa 2: T, rows 3: T, grid 4:mkw 6: T, g 7: T, e 8: f, T, p 9: T, s"
    );
    assert_eq!(
        marks(arrays, "definition-bodies,pointer-declarators"),
        "1:sa 2: T,mrows 3: T,mgrid 4:mkw 6: T,gg 7: T, e 8: f, T, p 9: T,gs"
    );
    // Shapes it does not read (a nested declarator, a word inside the group, a parameter
    // list on the next line, and the declarators inside that list); a K&R parameter's
    // declaration, a directive and an array's size; and a body the reference reads at
    // file scope, where a call in an array's size or after a `case` or `default` label
    // defines nothing.
    let unread = "struct u {\nT (*(*nested)(int))(double);\n};\n\
        int g(a, cb) int a; T (*cb)(); { return a; }\n\
        #pragma weak (*fp)(void)\nint arr[F (*probe)(int)];\nVALUE\n(lost)(long n)\n{\n\
        char buf[(*len)(n)];\nswitch (n) {\ncase 1: (*case_call)(n);\n\
        default: T (*default_call)(n);\n}\n}\n";
    assert_eq!(
        marks(unread, "definition-bodies,pointer-declarators"),
        marks(unread, "definition-bodies")
    );
    // After an invocation without `;`, a typedef's name stays a typedef.
    let invocation = "DECLARE_RUN_ONCE(c)\ntypedef void (*F)(void);\nint after;\n";
    assert_eq!(
        marks(invocation, "definition-bodies,pointer-declarators"),
        "1: DECLARE_RUN_ONCE, c 2:tF 3:gafter"
    );
}

#[test]
fn typedef_lists_restore_parameter_reading_after_macro_definitions() {
    let source = "DECLARE(x)\ntypedef void (*const alias)(int parameter);\n\
        typedef struct item *(*factory)(int param);\ntypedef int direct(int first,\n\
        #define HELP typedef void (*inner)(int arg);\n int value);\nint after;\n";
    for newline in ["\n", "\r\n"] {
        assert_eq!(
            marks(&source.replace('\n', newline), "enabled"),
            "1: DECLARE, x 2:talias, parameter 3: item,tfactory, param \
             4:tdirect, first 5:#HELP,tinner,targ 6: value 7:gafter"
        );
    }
}

#[test]
fn declarator_lists_end_across_conditional_arms_and_statement_boundaries() {
    let source = "struct ops {\nT (*read) /* gap */\n(int arg);\n#if A\n\
        int (*next)(int x) ATTR;\n#else\nint (*other)(int y) OTHER;\n#endif\n\
        int member;\n};\ntypedef int (alias)(int parameter);\nint after;\n";
    for newline in ["\n", "\r\n"] {
        assert_eq!(
            marks(&source.replace('\n', newline), "enabled"),
            "1:sops 2: T,mread 3: arg 4: A 5:mnext, x, ATTR \
             7:mother, y, OTHER 9:mmember 11:talias, parameter 12:gafter"
        );
    }
}

#[test]
fn split_pointer_declarations_define_names_not_parameters() {
    let source = "struct ops {\nT (*read)\n (struct buffer *p, int count);\n\
        int (*write)\n (T *item, void (*notify)(void *arg));\n};\n\
        T (*global)\n (int value);\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:sops 2: T,mread 3: buffer, p, count 4:mwrite \
         5: T, item, notify, arg 7: T,gglobal 8: value 9:gafter"
    );
    let source = "typedef struct ops {\nT (*read)\n(int arg);\n} ops;\n\
        T (*first)\n(int x), (*second)\n(int y);\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:sops 2: T,mread 3: arg 4:tops 5: T,gfirst 6: x,gsecond 7: y 8:gafter"
    );
}

#[test]
fn split_pointer_parameters_preserve_inline_aggregate_members() {
    let source = "struct ops {\nint (*read)\n\
        (struct param_tag { int real_member; } *p);\nint after;\n};\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:sops 2:mread 3:sparam_tag,mreal_member,mp 4:mafter"
    );
}

#[test]
fn split_pointer_parameters_preserve_an_enclosing_function_form() {
    let source = "void f(struct tag { int member; } value, T (*cb)\n\
        (int arg), int other);\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1: f,stag, member, value, T, cb 2: arg, other 3:gafter"
    );
    let parenthesised = source.replace("void f(", "void (f)(");
    assert!(!marks(&parenthesised, "enabled").contains("gcb"));
}

#[test]
fn split_pointer_parameters_leave_opaque_syntax_unchanged() {
    let source = "struct aliases {\nTYPEOF(x)\n(alias);\n\
        TYPEOF(*ptr)\n(other_alias);\nT (*opaque)\n(T *p);\n\
        T (*commented)\n(int /* note */ q);\nT (*rows)\n[4];\n};\n\
        typedef int (*callback)\n(int value);\nextern T (*external)\n(int ext_arg);\n\
        void owner(void) {\nT (*local)\n(int local_arg);\n}\nint\n\
        (wrapper)(int value)\n{ return choose(left *right)\n(value, value); }\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:saliases 2: TYPEOF, x 3:malias 4: TYPEOF, ptr 5:mother_alias \
         6: T, opaque 7: T,mp 8: T, commented 9:mq 10: T, rows \
         13:tcallback 14:tvalue 15: T, external 16: ext_arg 17:$owner \
         18:`T, local 19: local_arg 20:} 22:gwrapper, value \
         23: choose, left, right 24:gvalue"
    );
    for width in [256, 512] {
        for list in [
            format!("{}int value", " ".repeat(width)),
            format!("int {}value", " ".repeat(width)),
        ] {
            let source = format!("struct bounds {{\nT (*far)\n({list});\n}};\n");
            assert_eq!(marks(&source, "enabled"), "1:sbounds 2: T, far 3:mvalue");
        }
    }
}

#[test]
fn decorated_pointer_names_are_definitions() {
    let source = "T (CALLBACK *handler)(int arg);\nstruct ops {\n\
        HRESULT (STDMETHODCALLTYPE __RPC_FAR *query)(void *self);\n\
        T (API * const *table[4])(int value);\n\
        T (API *multiple)(T *first, U *second);\n\
        T (API *empty)();\n};\n\
        extern T (WINAPI *external)(void);\n";
    assert_eq!(
        marks(source, "enabled"),
        "1: T, CALLBACK,ghandler, arg 2:sops \
         3: HRESULT, STDMETHODCALLTYPE, __RPC_FAR,mquery, self \
         4: T, API,mtable, value 5: T, API,mmultiple, first, U, second \
         6: T, API,mempty 8: T, WINAPI, external"
    );
}

#[test]
fn decorated_pointers_preserve_type_expressions_and_declaration_boundaries() {
    let source = "int left, right;\n__typeof__(left *right) (product);\n\
        TYPEOF(left *right) (macro_product);\n\
        TYPEOF(left *right) (*pointer_product);\n\
        TYPEOF(left *right) (array_product[(1, 2)]);\n\
        T (API opaque)(int arg);\nT (*AFTER opaque_after)(int arg);\n\
        T (*BETWEEN *opaque_between)(int arg);\n\
        T (API *opaque_parameter)(T);\n\
        T (API *comment_parameter)(/* comma, */ T);\n\
        extern T (CALLBACK *external)(int arg);\n\
        int x = invoke(API *operand)(int_value, other);\n\
        void owner(T (CALLBACK *parameter)(int)) {\n\
        T (CALLBACK *local)(int arg);\n}\n\
        #define DECL T (CALLBACK *macro_name)(int arg)\n";
    let without = Corrections::ENABLED
        .names()
        .replace(",pointer-declarators", "");
    assert_eq!(marks(source, "enabled"), marks(source, &without));
    // A body whose parenthesised head the reference misses is still not file scope.
    let missed_body = "typedef int callback(int, int);\ncallback *choose(int value);\n\
        int left, right;\nint\n(wrapper)(int value)\n\
        { return choose(left *right)(value, value); }\n";
    assert_eq!(marks(missed_body, "enabled"), marks(missed_body, &without));
    for blanks in [256, 512] {
        let source = format!("T (API *far)(T{}first, U second);\n", " ".repeat(blanks));
        assert_eq!(marks(&source, "enabled"), marks(&source, &without));
        let source = format!("T (API{} *far)(int arg);\n", "X".repeat(blanks));
        assert_eq!(marks(&source, "enabled"), marks(&source, &without));
    }
}

#[test]
fn function_typedef_parameters_are_occurrences() {
    let source = "typedef int callback(int count, const T *item);\n\
        typedef void (*handler)(int code, void (*nested)(int value));\n\
        typedef int first(int a), second(int b);\n\
        typedef void (CALLBACK *decorated)(int arg);\n\
        typedef int scalar;\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:tcallback, count, T, item 2:thandler, code, nested, value \
         3:tfirst, a,tsecond, b \
         4: CALLBACK,tdecorated, arg 5:tscalar 6:gafter"
    );
}

#[test]
fn pointer_lists_keep_comment_gaps_and_suffix_annotations_in_the_declaration() {
    let source = "int *(*first)(int a) ATTR, (*second)(int b) OTHER;\n\
        int after;\nstruct ops {\n\
        void (*reset) /* comment */\n(fsm *);\n\
        int (*next) /* comment */ (fsm *, int *code) ATTR;\n};\n\
        int (*x)/*gap*/(void); int count=(x=1);\n\
        API void * (*Alloc)(size_t bytes) ATTR;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:gfirst, a, ATTR,gsecond, b, OTHER 2:gafter 3:sops \
         4:mreset 5: fsm 6:mnext, fsm, code, ATTR 8:gx,gcount, x \
         9: API,gAlloc, size_t, bytes, ATTR"
    );
    let without = Corrections::ENABLED
        .names()
        .replace(",pointer-declarators", "");
    for source in [
        "T (*opaque) /* gap */ (Alias);\nint after;\n".to_string(),
        "TYPEOF(*ptr) /* gap */ (DECOR *global_alias);\nint after;\n".to_string(),
        "T (*aggregate) /* gap */ (struct S { int field; } arg);\nint after;\n".to_string(),
        "T (*unclosed) /* gap */ (int arg;\nint after;\n".to_string(),
        "int (*spliced) /* *\\\n/ (void);\nint after;\n".to_string(),
        format!(
            "T (*distant) /* {} */ (int arg);\nint after;\n",
            "x".repeat(256)
        ),
    ] {
        assert_eq!(
            marks(&source, "enabled"),
            marks(&source, &without),
            "{source}"
        );
    }
}

#[test]
fn parenthesised_function_typedefs_keep_only_the_alias_as_a_definition() {
    let source = "typedef void (__cdecl *sig_handler_t)(int);\n\
        typedef unsigned int (WINAPI *IF_NAMETOINDEX_FN)(const char *);\n\
        typedef sds (aux_value_getter) (clusterNode* n, sds s);\n\
        typedef int (aux_value_present) (clusterNode* n);\n\
        typedef T (a)(int x), (b)(int y);\n\
        typedef enum cb_next (*cb_iter)(struct cb_node *, void *arg);\n\
        typedef enum rewrite_result (*rewrite_parent_fn_t)(struct rev_info *revs, struct commit **pp);\n\
        typedef union u *(*fp)(int x);\n\
        typedef T (*unknown)(int arg);\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1: __cdecl,tsig_handler_t 2: WINAPI,tIF_NAMETOINDEX_FN \
         3: sds,taux_value_getter, clusterNode, n, s \
         4:taux_value_present, clusterNode, n 5: T,ta, x,tb, y \
         6: cb_next,tcb_iter, cb_node, arg \
         7: rewrite_result,trewrite_parent_fn_t, rev_info, revs, commit, pp \
         8: u,tfp, x 9: T,tunknown, arg 10:gafter"
    );
}

#[test]
fn opaque_pointer_macro_in_a_typed_group_does_not_define_an_alias() {
    let source = "typedef void (APIENTRYP callback)(GLfloat n, GLfloat f);\n\
        typedef int (ABI query)(Other *value);\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1: APIENTRYP,tcallback, GLfloat, n, f 2: ABI,tquery, Other, value 3:gafter"
    );
}

#[test]
fn typedef_parameters_preserve_declarations_and_ambiguous_type_macros() {
    let controls = "int *ptr;\ntypedef __typeof__(*ptr) (alias);\n\
        #define TYPEOF(x) __typeof__(x)\ntypedef TYPEOF(*ptr) (macro_alias);\n\
        typedef int API (*x)(void);\n\
        typedef int API (bare_alias);\ntypedef int API (/* comment */ *comment_alias)(void);\n\
        typedef int API (int_alias);\ntypedef int API (void_alias);\n\
        typedef int API (*const const_alias)(void);\n\
        typedef int (*factory(int input))(int output);\n";
    let without = Corrections::ENABLED
        .names()
        .replace(",typedef-parameters", "");
    assert_eq!(marks(controls, "enabled"), marks(controls, &without));
    let conditional_tag = "#if ENABLE_TAG\ntypedef struct ConditionalTag\n\
        #else\ntypedef struct\n#endif\n{ int member; } ConditionalAlias;\n";
    assert_eq!(
        marks(conditional_tag, "enabled"),
        marks(conditional_tag, &without)
    );
    assert!(marks(conditional_tag, "enabled").contains("gConditionalTag"));
    // Type macros followed by parenthesised aliases remain declarations, not lists.
    for source in [
        "typedef TYPEOF(*ptr) (alias);\n",
        "typedef TYPEOF(*ptr) (*alias);\n",
        "typedef TYPEOF(*ptr) (API alias);\n",
        "typedef TYPEOF(*ptr) (API *pointer_alias);\n",
        "typedef TYPEOF (ptr) (API alias);\n",
        "typedef TYPEOF(left *right) (alias);\n",
        "typedef TYPEOF(left *right) (*alias);\n",
        "typedef T (*opaque)(Other);\n",
        "typedef T (*commented)(/* note */ int arg);\n",
        "typedef T (*aggregate)(struct S { int member; } arg);\n",
    ] {
        assert_eq!(
            marks(source, "enabled"),
            marks(source, &without),
            "{source}"
        );
    }
    let source = "typedef int callback(\nint count,\n\
        struct tag { int member; } value);\ntypedef int next;\n\
        void run(void) { typedef int local(int arg); }\nint after;\n";
    assert_eq!(
        marks(source, "enabled"),
        "1:tcallback 2: count 3:stag, member, value 4:tnext \
         5:$run,tlocal, arg,} 6:gafter"
    );
    let declarations = "typedef int f(int x), (*g)(int y), h;\n";
    assert_eq!(marks(declarations, "enabled"), "1:tf, x,tg, y,th");
    assert_eq!(
        marks("typedef int same(int same);\n", "enabled"),
        "1:tsame, same"
    );
    // A window must not turn the tail of an opaque type macro into a keyword.
    for stars in 245..=260 {
        let source = format!("typedef TYPE_int({}ptr) (alias);\n", "*".repeat(stars));
        assert_eq!(marks(&source, "enabled"), marks(&source, &without));
    }
    for blanks in 248..=260 {
        let source = format!("typedef int API ({}int_alias);\n", " ".repeat(blanks));
        assert_eq!(marks(&source, "enabled"), marks(&source, &without));
        let source = format!("typedef T (*long_list)(T *{}arg);\n", " ".repeat(blanks));
        assert_eq!(marks(&source, "enabled"), marks(&source, &without));
    }
    assert_eq!(
        marks(
            "typedef int qualified(const volatile char *path);\n",
            "enabled"
        ),
        "1:tqualified, path"
    );
}

/// A keyword that ends definition-bodies' function-declaration form is also the type a
/// pending `typedef` waits for, so the typedef after an invocation line without `;`
/// defines its name as it does without the Correction.
#[test]
fn a_typedef_after_an_invocation_keeps_its_name() {
    let source = "DECLARE_RUN_ONCE(c)\ntypedef void (*F)(void);\nint after;\n\
        OBJECT_DECLARE_TYPE(A, B, C)\ntypedef void (Setup) (int nf);\n\
        BEGIN()\ntypedef int glob_func(const char *path);\n\
        libc_hidden_proto (timer)\ntypedef int kernel_timer_t;\nint last;\n";
    let typedefs = |corrections: &str| -> Vec<String> {
        let corrections = super::Corrections::parse(corrections).expect("corrections");
        super::scan_with(source.as_bytes(), Mode::C, corrections)
            .iter()
            .flat_map(|record| {
                record
                    .symbols
                    .iter()
                    .filter(|symbol| symbol.mark.byte() == b't')
                    .map(|symbol| {
                        String::from_utf8_lossy(&record.text[symbol.at..symbol.at + symbol.len])
                            .into_owned()
                    })
            })
            .collect()
    };
    let reference = typedefs("");
    for name in ["F", "Setup", "glob_func", "kernel_timer_t"] {
        assert!(
            reference.iter().any(|symbol| symbol == name),
            "{reference:?}"
        );
    }
    assert_eq!(typedefs("definition-bodies"), reference);
    assert_eq!(
        typedefs("enabled"),
        ["F", "Setup", "glob_func", "kernel_timer_t"]
    );
}

/// parenthesised-typedefs: a typedef's type keyword that the reference reads as a call
/// head (the definition shape rejects what follows the group) is the type when the group
/// after it holds only the declarator's name, so that name is the typedef name, not a
/// global. A word before the name, a name on the next line and a nested declarator keep
/// the reference's reading [42-parenthesised-typedefs].
#[test]
fn a_typedef_of_a_parenthesised_declarator_defines_its_name() {
    let source = "typedef void (*f)();\ntypedef int (name)();\ntypedef int (*A)[4];\n\
        typedef int ((*e) (F, int));\ntypedef int (*cb)(/* int c */);\n\
        typedef void (^blk)();\ntypedef void (*two)(), (*three)();\n\
        typedef void ( __interrupt __far * isr )();\ntypedef char (*\nsplit)();\n\
        typedef int (*nested(int))();\nint g;\n";
    let reference = "1:gf 2:gname 3:gA 4:ge,tF 5:gcb 6:gblk 7:gtwo,tthree \
        8: __interrupt,t__far,tisr 10:gsplit 11: nested 12:gg";
    let corrected = "1:tf 2:tname 3:tA 4:te,tF 5:tcb 6:tblk 7:ttwo,tthree \
        8: __interrupt,t__far,tisr 10:gsplit 11: nested 12:gg";
    assert_eq!(marks(source, ""), reference);
    assert_eq!(marks(source, "parenthesised-typedefs"), corrected);
    assert_eq!(
        marks(source, "enabled"),
        "1:tf 2:tname 3:tA 4:te, F 5:tcb 6:tblk 7:ttwo,tthree \
         8: __interrupt, __far,tisr 10:gsplit 11: nested 12:gg"
    );
    // Where no typedef waits for its type, or the keyword is not read as a call head,
    // nothing changes.
    let others = "int (*g1)();\ntypedef void (*withparams)(int n);\n\
        void run(void) { typedef void (*local)(); }\n#define D typedef void (*m)();\n";
    assert_eq!(
        marks(others, "enabled"),
        marks(
            others,
            &Corrections::ENABLED
                .names()
                .replace(",parenthesised-typedefs", "")
        )
    );
}

/// typedef-qualifiers (T1): a qualifier right after
/// `typedef` is not its type, so the type name after it is a plain name, not a typedef
/// name, even where the reference's shapes would make it a global (`object_id` before
/// `*(*fn)`) [43-typedef-qualifiers]. A name that `;`, `,`, `[` or `)` follows is the
/// declarator (an implicit `int`, or `k` in a group); a tag being defined leaves the
/// decision to the next name, and its body is the type, so a declarator after an empty
/// body keeps its name. A macro's name read inside the typedef is never made plain.
#[test]
fn a_qualifier_is_not_a_typedefs_type() {
    let source = "typedef const foo_t bar_t;\ntypedef volatile struct tag *p;\n\
        typedef const char *cstr;\ntypedef void (* const k)();\ntypedef const x_t;\n\
        typedef const struct object_id *(*fn)(void *);\ntypedef const struct e {} e_t;\n\
        typedef const struct f {} f_t(void);\n";
    assert_eq!(
        marks(source, ""),
        "1:tfoo_t,tbar_t 2:ttag,tp 3:tcstr 4:tk 5:tx_t 6:tobject_id,tfn 7:se,te_t 8:sf,tf_t"
    );
    let corrected =
        "1: foo_t,tbar_t 2: tag,tp 3:tcstr 4:tk 5:tx_t 6: object_id,tfn 7:se,te_t 8:sf,tf_t";
    assert_eq!(marks(source, "typedef-qualifiers"), corrected);
    assert_eq!(marks(source, "enabled,typedef-qualifiers"), corrected);
    // Alone, without macro-bodies: the macro keeps a definition's mark.
    let define = "typedef const\n#define INNER 1\nfoo_t after_t;\n";
    assert_eq!(marks(define, ""), "2:tINNER 3:tfoo_t,tafter_t");
    assert_eq!(
        marks(define, "typedef-qualifiers"),
        "2:#INNER 3:tfoo_t,tafter_t"
    );
}

/// extern-definitions: an `extern` before a function definition ends with its body. The
/// reference keeps it until a `;` at its depth, so the next declaration defines nothing;
/// with macro-bodies alone a `;` in a macro no longer ends it either, and the members
/// after the macro are lost too [44-extern-definitions].
#[test]
fn an_extern_before_a_definition_ends_with_its_body() {
    let source = "extern void f(int x)\n{\n  g(x);\n}\n#define M int y = 0; (void)y\n\
        typedef struct {\n  const char* s;\n  const char* end;\n} key;\nint after;\n\
        extern int h(void) { return 0; }\nint lost;\nint kept;\n";
    let reference = "1:$f, x 3:`g, x 4:} 5:#M, y,) 7:ms 8:mend 9:tkey 10:gafter 11:$h,} \
        12: lost 13:gkept";
    let corrected = "1:$f, x 3:`g, x 4:} 5:#M, y,) 7:ms 8:mend 9:tkey 10:gafter 11:$h,} \
        12:glost 13:gkept";
    assert_eq!(marks(source, ""), reference);
    assert_eq!(marks(source, "extern-definitions"), corrected);
    assert_eq!(
        marks(source, "macro-bodies"),
        "1:$f, x 3:`g, x 4:} 5:#M, y,) 7: s 8: end 9:tkey 10:gafter 11:$h,} 12: lost \
         13:gkept"
    );
    assert_eq!(marks(source, "macro-bodies,extern-definitions"), corrected);
    assert_eq!(marks(source, "enabled"), corrected);
    // An extern declaration still ends at its `;`, and an `extern` inside a body is the
    // body's own.
    let declarations = "extern int e;\nint g1;\nextern int p(void);\nint g2;\n\
        int f(void) { extern int inner; return inner; }\nint g3;\n";
    assert_eq!(
        marks(declarations, "extern-definitions"),
        marks(declarations, "")
    );
}

/// definition-bodies in a linkage block: a head without a body that begins its
/// declaration is a macro among the specifiers, and the name after it declares. One brace
/// deep in `extern "C" {`, once an inner `extern ...;` has ended its `extern` state, the
/// reference reads such heads as calls and defines the names; outside a linkage block it
/// takes them for function definitions, and a head there still reads as a function
/// declaration [45-specifier-macros].
#[test]
fn a_macro_that_begins_a_declaration_in_a_linkage_block_declares_the_name() {
    let source = "#ifdef __cplusplus\nextern \"C\" {\n#endif\nextern int f(int x);\n\
        PyAPI_DATA(Py_ssize_t) _Py_RefTotal;\nPyAPI_DATA(const struct _frozen *) frozen;\n\
        PyAPI_DATA(int) table[];\nPyAPI_FUNC(int) PyFoo(int x);\nchar *g(int n) FAST_FUNC;\n\
        #ifdef __cplusplus\n}\n#endif\nPyAPI_DATA(int) outside;\nint after;\n";
    assert_eq!(
        marks(source, ""),
        "1: __cplusplus 4: f, x 5: PyAPI_DATA, Py_ssize_t,g_Py_RefTotal \
         6: PyAPI_DATA, _frozen,gfrozen 7: PyAPI_DATA,gtable 8: PyAPI_FUNC, PyFoo, x \
         9: g, n,gFAST_FUNC 10: __cplusplus 13:$PyAPI_DATA, outside 14: after"
    );
    // The same globals, but a word after a declared function's list stays plain.
    assert_eq!(
        marks(source, "linkage-blocks,definition-bodies"),
        "1: __cplusplus 4: f, x 5: PyAPI_DATA, Py_ssize_t,g_Py_RefTotal \
         6: PyAPI_DATA, _frozen,gfrozen 7: PyAPI_DATA,gtable 8: PyAPI_FUNC, PyFoo, x \
         9: g, n, FAST_FUNC 10: __cplusplus 13: PyAPI_DATA, outside 14:gafter"
    );
    // Known limits, read as the reference reads them one brace deep: a word after an
    // implicit `int` prototype's list, an attribute after the declarator and a tag's
    // forward declaration are globals; after two macros in a row the name stays plain,
    // where the reference defines it.
    let limits = "extern \"C\" {\nextern int f(int x);\nimplicit(int x) ATTR2;\n\
        M(T) name_attr ATTR4;\nM(x) struct tagfwd;\nA(x) B(y) twomac;\n}\n";
    assert_eq!(
        marks(limits, ""),
        "2: f, x 3: implicit, x,gATTR2 4: M, T, name_attr,gATTR4 5: M, x,gtagfwd \
         6: A, x, B, y,gtwomac"
    );
    assert_eq!(
        marks(limits, "linkage-blocks,definition-bodies"),
        "2: f, x 3: implicit, x,gATTR2 4: M, T, name_attr,gATTR4 5: M, x,gtagfwd \
         6: A, x, B, y, twomac"
    );
}

/// definition-bodies in a linkage block with no inner `extern` declaration, where the
/// reference's `extern` state from `extern "C"` hides the first declaration: a head that
/// begins its declaration right after the `{`, after a directive line, after an inline
/// function's body and after an initializer is a macro among the specifiers. A directive's
/// words and a body's or an initializer's braces do not begin the next declaration.
#[test]
fn a_macro_that_begins_a_declaration_anywhere_in_a_linkage_block_declares_the_name() {
    let source = "extern \"C\" {\nPyAPI_DATA(int) first;\n#if defined(X)\n\
        PyAPI_DATA(int) after_if;\n#endif\nstatic inline int f(void) { return 0; }\n\
        PyAPI_DATA(int) after_body;\nM(T) initialised = {0};\nPyAPI_DATA(int) after_init;\n\
        }\nint last;\n";
    // The reference defines nothing in the block, nor the name after it: its `extern`
    // state lasts to the first `;` at file scope.
    assert_eq!(
        marks(source, ""),
        "2: PyAPI_DATA, first 3: defined, X 4: PyAPI_DATA, after_if 6: inline, f \
         7: PyAPI_DATA, after_body 8: M, T, initialised 9: PyAPI_DATA, after_init 11: last"
    );
    assert_eq!(
        marks(source, "linkage-blocks,definition-bodies"),
        "2: PyAPI_DATA,gfirst 3: defined, X 4: PyAPI_DATA,gafter_if 6: inline,$f,} \
         7: PyAPI_DATA,gafter_body 8: M, T,ginitialised 9: PyAPI_DATA,gafter_init 11:glast"
    );
}

/// definition-bodies: an attribute head without a body is a macro among the specifiers
/// anywhere, so the name after it declares. The reference takes `__declspec` there for a
/// function definition that owns what follows [45-specifier-macros].
#[test]
fn an_attribute_before_a_declarator_declares_the_name_after_it() {
    let source = "static __declspec(thread) struct state tls;\n\
        __declspec(selectany) T *global_ptr;\n__declspec(dllexport) int exported(int a);\n\
        extern __declspec(dllimport) T imported;\nint after;\n";
    assert_eq!(
        marks(source, ""),
        "1:$__declspec, thread, state, tls 2:$__declspec, selectany, T, global_ptr \
         3:$__declspec, dllexport,`exported, a 4:$__declspec, dllimport, T, imported \
         5: after"
    );
    // A prototype after the attribute, and an extern declaration, still define nothing.
    assert_eq!(
        marks(source, "definition-bodies"),
        "1: __declspec, thread, state,gtls 2: __declspec, selectany, T,gglobal_ptr \
         3: __declspec, dllexport, exported, a 4: __declspec, dllimport, T, imported \
         5:gafter"
    );
}

/// An attribute before a function body belongs to its declaration, so the real head
/// defines the function and owns the calls in its body.
#[test]
fn an_attribute_before_a_function_body_keeps_the_real_function() {
    let source = "__declspec(dllexport) int main(void) { run(); }\n\
        int __declspec(noinline) VimMain(void) { return work(); }\n\
        __declspec(dllexport) BOOL WINAPI DllMain(void) { enter(); }\n\
        int after;\n";
    let expected = "1: __declspec, dllexport,$main,`run,} \
        2: __declspec, noinline,$VimMain,`work,} \
        3: __declspec, dllexport, BOOL, WINAPI,$DllMain,`enter,} 4:gafter";
    assert_eq!(marks(source, "definition-bodies"), expected);
    assert_eq!(marks(source, "enabled"), expected);
}

/// Each name looks a fixed distance back and ahead, so a line of many qualified groups
/// costs linear time; a group that reaches farther is not read.
#[test]
fn pointer_declarators_look_a_bounded_distance() {
    let read = |qualifiers: usize, dimensions: usize| {
        let source = format!(
            "struct s {{\nT (*{}m{})(int);\n}};\n",
            "const ".repeat(qualifiers),
            "[]".repeat(dimensions)
        );
        marks(&source, "definition-bodies,pointer-declarators")
    };
    assert_eq!(read(40, 50), "1:ss 2: T,mm");
    assert_eq!(read(60, 0), "1:ss 2: T, m");
    assert_eq!(read(0, 150), "1:ss 2: T, m");
}

/// Reports the files whose look-ahead work limits act on the enabled reading: for each
/// path listed in the file `RIPSCOPE_COST_LIST`, the heads read with the narrow window,
/// the groups closing-arms left unread, and the lines whose records differ without the
/// limits. Prints one tab-separated line per affected file and a total. Run it over the
/// corpus with
/// `RIPSCOPE_COST_LIST=list cargo test --release --bin ripscope work_limit_hits -- --ignored --nocapture`.
#[test]
#[ignore = "measures a corpus"]
fn work_limit_hits() {
    let list = std::env::var("RIPSCOPE_COST_LIST").expect("RIPSCOPE_COST_LIST names a file list");
    let list = std::fs::read_to_string(list).expect("file list");
    let (mut files, mut hit, mut changed) = (0, 0, 0);
    for path in list.lines().filter(|line| !line.is_empty()) {
        let Ok(bytes) = std::fs::read(path) else {
            println!("unreadable\t{path}");
            continue;
        };
        files += 1;
        let mode = Mode::for_name(path.as_bytes());
        let (records, cost) = super::scan_with_cost(&bytes, mode, Corrections::ENABLED);
        if cost.narrow_windows == 0 && cost.closing_refusals == 0 {
            continue;
        }
        hit += 1;
        let unbounded = super::scan_unbounded(&bytes, mode, Corrections::ENABLED);
        let lines: std::collections::BTreeSet<u32> = records
            .iter()
            .zip(&unbounded)
            .filter(|(limited, free)| limited != free)
            .map(|(limited, _)| limited.line)
            .collect();
        let differs = !lines.is_empty() || records.len() != unbounded.len();
        changed += usize::from(differs);
        println!(
            "hit\t{path}\t{}\t{}\t{}\t{}\t{:?}",
            bytes.len(),
            cost.narrow_windows,
            cost.closing_refusals,
            usize::from(differs),
            lines.iter().take(5).collect::<Vec<_>>()
        );
    }
    println!("total\tfiles {files}\thit {hit}\tchanged {changed}");
}
