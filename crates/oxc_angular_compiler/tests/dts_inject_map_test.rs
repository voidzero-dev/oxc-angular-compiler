//! Source maps for the `.d.ts` injection pass (#693): original tokens keep
//! their mapping at their new place and injected text is unmapped, as in
//! ngtsc's `afterDeclarations` emit.

use oxc_angular_compiler::{
    DtsInjectDeclaration, inject_dts_declarations, inject_dts_declarations_with_identity_map,
    inject_dts_declarations_with_map,
};

fn decl(class_name: &str) -> DtsInjectDeclaration {
    DtsInjectDeclaration {
        class_name: class_name.to_string(),
        members: format!(
            "static ɵfac: i0.ɵɵFactoryDeclaration<{class_name}, never>;\n\
             static ɵcmp: i0.ɵɵComponentDeclaration<{class_name}, \"x\", never, {{}}, {{}}, never, never, true, never>;"
        ),
        namespace_imports: [("i0".to_string(), "@angular/core".to_string())].into_iter().collect(),
        source_file: None,
    }
}

const BASE64: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Decode `mappings` into absolute segments per line.
fn decode(mappings: &str) -> Vec<Vec<Vec<i64>>> {
    let mut state = [0i64; 5];
    mappings
        .split(';')
        .map(|line| {
            state[0] = 0;
            line.split(',')
                .filter(|s| !s.is_empty())
                .map(|segment| {
                    let mut fields = Vec::new();
                    let (mut value, mut shift) = (0i64, 0);
                    for byte in segment.bytes() {
                        let digit = BASE64.bytes().position(|b| b == byte).unwrap() as i64;
                        value |= (digit & 31) << shift;
                        if digit & 32 == 0 {
                            let v = if value & 1 == 1 { -(value >> 1) } else { value >> 1 };
                            let i = fields.len();
                            state[i] += v;
                            fields.push(state[i]);
                            (value, shift) = (0, 0);
                        } else {
                            shift += 5;
                        }
                    }
                    fields
                })
                .collect()
        })
        .collect()
}

/// The original `(line, col)` a generated position maps to, like
/// trace-mapping's `originalPositionFor` (the last segment at or before
/// `col` on that line).
fn lookup(decoded: &[Vec<Vec<i64>>], line: usize, col: i64) -> Option<(i64, i64)> {
    let segment = decoded.get(line)?.iter().rev().find(|s| s[0] <= col)?;
    (segment.len() >= 4).then(|| (segment[2], segment[3]))
}

fn utf16_lines(text: &str) -> Vec<Vec<u16>> {
    text.split('\n').map(|line| line.encode_utf16().collect()).collect()
}

/// UTF-16 column of every token start: identifier runs and other non-space
/// chars.
fn token_starts(line: &[u16]) -> Vec<usize> {
    let chars: Vec<(usize, char)> = {
        let mut col = 0;
        char::decode_utf16(line.iter().copied())
            .map(|c| {
                let c = c.unwrap();
                let at = col;
                col += c.len_utf16();
                (at, c)
            })
            .collect()
    };
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut starts = Vec::new();
    let mut prev_word = false;
    for (col, c) in chars {
        if !(c.is_whitespace() || (word(c) && prev_word)) {
            starts.push(col);
        }
        prev_word = word(c);
    }
    starts
}

/// Every original token is mapped exactly once, from a spot in `code` that
/// holds the same text, and every mapped span of `code` is a copy of the
/// original (so no injected text is mapped).
fn assert_identity(source: &str, code: &str, mappings: &str) {
    let decoded = decode(mappings);
    let original = utf16_lines(source);
    let generated = utf16_lines(code);
    let mut mapped: Vec<(i64, i64)> = Vec::new();
    for (line, segments) in decoded.iter().enumerate() {
        for (i, segment) in segments.iter().enumerate() {
            if segment.len() < 4 {
                continue;
            }
            assert_eq!(segment[1], 0, "one source");
            mapped.push((segment[2], segment[3]));
            // Up to the next segment (or the line end), the code is a copy.
            let end = segments.get(i + 1).map_or(generated[line].len() as i64, |s| s[0]);
            let from = &generated[line][segment[0] as usize..end as usize];
            let to = &original[segment[2] as usize][segment[3] as usize..];
            assert!(
                to.starts_with(from),
                "line {line} col {} maps to {:?} but reads {:?} vs {:?}\n{code}",
                segment[0],
                (segment[2], segment[3]),
                String::from_utf16_lossy(from),
                String::from_utf16_lossy(&to[..from.len().min(to.len())]),
            );
        }
    }
    let expected: Vec<(i64, i64)> = original
        .iter()
        .enumerate()
        .flat_map(|(line, text)| {
            token_starts(text).into_iter().map(move |col| (line as i64, col as i64))
        })
        .collect();
    assert_eq!(mapped, expected, "{code}");
}

#[test]
fn shifts_the_rolldown_plugin_dts_fixture() {
    // From a real rolldown 1.3.1 + rolldown-plugin-dts 0.30.0 build.
    let source = "export declare class HelloComponent {\n  name: string;\n}\n\
                  export declare class OtherComponent {\n  value: number;\n  greet(): string;\n}\n\
                  export declare class EmptyOne {}\n//# sourceMappingURL=index.d.mts.map";
    let declarations = [decl("HelloComponent"), decl("OtherComponent"), decl("EmptyOne")];
    let result = inject_dts_declarations_with_map(
        source,
        &declarations,
        Some("A,qBAAa;EACX;;qBAGW;EACX;EACA;;qBAEW"),
    );
    assert_eq!(result.code, inject_dts_declarations(source, &declarations));
    assert_eq!(result.mappings.as_deref(), Some(";A,qBAAa;EACX;;;;qBAGW;EACX;EACA;;;;qBAEW"));
}

#[test]
fn moves_a_mid_line_closing_brace_to_its_own_line() {
    let source = "export declare class X {}\n";
    // `X` at 21, `{` at 23, `}` at 24.
    let result = inject_dts_declarations_with_map(source, &[decl("X")], Some("qBAAA,EAAE,CAAC"));
    let code = &result.code;
    let decoded = decode(result.mappings.as_deref().unwrap());
    // Line 0 is the import, line 1 the class head, 2-3 the members.
    assert_eq!(decoded[0], Vec::<Vec<i64>>::new());
    assert_eq!(decoded[1], vec![vec![21, 0, 0, 0], vec![23, 0, 0, 2]]);
    assert_eq!(decoded[4], vec![vec![0, 0, 0, 3]]);
    assert_eq!(code.lines().nth(4), Some("}"));
    assert_eq!(lookup(&decoded, 2, 4), None);

    let identity = inject_dts_declarations_with_identity_map(source, &[decl("X")]);
    assert_eq!(&identity.code, code);
    assert_identity(source, code, &identity.mappings.unwrap());
}

#[test]
fn moves_code_after_an_import_on_the_same_line() {
    let source = "import { Y } from \"y\"; export declare class X { y: Y; }\n";
    let result = inject_dts_declarations_with_identity_map(source, &[decl("X")]);
    let code = &result.code;
    assert!(code.starts_with("import { Y } from \"y\";\nimport * as i0"), "{code}");
    let decoded = decode(result.mappings.as_deref().unwrap());
    // The first import stays, `export` goes to (2, 1), the import line is
    // unmapped.
    assert_eq!(lookup(&decoded, 0, 0), Some((0, 0)));
    assert_eq!(lookup(&decoded, 0, 18), Some((0, 18)));
    assert_eq!(lookup(&decoded, 1, 0), None);
    assert_eq!(lookup(&decoded, 2, 1), Some((0, 23)));
    assert_identity(source, code, result.mappings.as_deref().unwrap());
}

#[test]
fn maps_the_jsdoc_below_the_new_import() {
    // The JSDoc of the first statement keeps its mapping at its new place,
    // below the import; the import line is unmapped.
    let source = "/** d */ export declare class X {\n}\n";
    let result = inject_dts_declarations_with_map(source, &[decl("X")], Some("AAAA"));
    assert!(result.code.starts_with("import * as i0 from \"@angular/core\";\n/** d */ export"));
    let decoded = decode(result.mappings.as_deref().unwrap());
    assert_eq!(decoded[0], Vec::<Vec<i64>>::new());
    assert_eq!(decoded[1], vec![vec![0, 0, 0, 0]]);

    let identity = inject_dts_declarations_with_identity_map(source, &[decl("X")]);
    let decoded = decode(identity.mappings.as_deref().unwrap());
    assert_eq!(lookup(&decoded, 0, 0), None);
    assert_eq!(lookup(&decoded, 1, 0), Some((0, 0)));
    assert_eq!(lookup(&decoded, 1, 9), Some((0, 9)));
    assert_identity(source, &identity.code, identity.mappings.as_deref().unwrap());
}

/// The import ngtsc adds to a file with no imports goes above the comments
/// of the first statement, so its JSDoc stays on the class (and a bundler
/// that drops the import keeps it). Only `/// <reference>` directives stay
/// on top; ngc puts the import above a license header too.
#[test]
fn puts_the_import_above_the_jsdoc_of_the_first_class() {
    let import = "import * as i0 from \"@angular/core\";\n";
    let cases = [
        // ngc: `import`, `/** Doc for H. */`, `export declare class H`.
        ("/** Doc for H. */\nexport declare class H {\n}\n", 0),
        ("// header\n/** Doc for H. */\nexport declare class H {\n}\n", 0),
        ("/*! pinned */\n/** Doc for H. */ export declare class H {\n}\n", 0),
        ("/// <reference types=\"node\" />\n/** Doc for H. */\nexport declare class H {\n}\n", 31),
        // ngc: `import`, `/* license */`, `/** Doc for H. */`, `export declare class H`.
        ("/* license */\n\n/** Doc for H. */\nexport declare class H {\n}\n", 0),
        ("/** Doc for H. */\n\nexport declare class H {\n}\n", 0),
        ("/* a */\n\n/* b */\n\n/** Doc for H. */\nexport declare class H {\n}\n", 0),
        (
            "/// <reference types=\"node\" />\n/// <reference lib=\"dom\" />\n\
             // header\n\n// more\n/** Doc for H. */\nexport declare class H {\n}\n",
            59,
        ),
        ("/// <reference types=\"node\" />\nexport declare class H {\n}\n", 31),
        ("/* license */\n\nexport declare class H {\n}\n", 0),
    ];
    for (source, at) in cases {
        let result = inject_dts_declarations_with_identity_map(source, &[decl("H")]);
        let code = &result.code;
        assert_eq!(&code[..at], &source[..at], "{code}");
        assert!(code[at..].starts_with(import), "{code}");
        assert!(code[at + import.len()..].starts_with(&source[at..source.find(" {").unwrap()]));
        assert_identity(source, code, result.mappings.as_deref().unwrap());
    }
}

/// The map shift is linear in the file and the splices: a 2 MB `.d.ts`
/// with 11000 classes, on many lines and on one line, takes well under a
/// second in release (it took minutes when every splice rescanned the file
/// up to it).
#[test]
fn shifts_the_map_of_a_large_file_quickly() {
    let classes = 11_000;
    let names: Vec<String> = (0..classes).map(|i| format!("Component{i}")).collect();
    let declarations: Vec<DtsInjectDeclaration> = names.iter().map(|name| decl(name)).collect();
    for separator in ["\n", " "] {
        let source: String = names
            .iter()
            .map(|name| {
                format!(
                    "/** The {name}. */{separator}export declare class {name} {{{separator}    \
                     value: string;{separator}    other(x: number): Record<string, number>;\
                     {separator}    readonly items: ReadonlyArray<{{ id: number; label: string }}>;\
                     {separator}}}{separator}"
                )
            })
            .collect();
        assert!(source.len() > 2_000_000, "{}", source.len());
        let start = std::time::Instant::now();
        let result = inject_dts_declarations_with_identity_map(&source, &declarations);
        let elapsed = start.elapsed();
        assert_eq!(result.code.matches("static ɵfac").count(), classes);
        assert_identity(&source, &result.code, result.mappings.as_deref().unwrap());
        assert!(elapsed.as_secs() < 20, "took {elapsed:?}");
    }
}

#[test]
fn counts_columns_in_utf16() {
    // `é` and `—` are 1 UTF-16 unit each (2 and 3 bytes), `𝒳` is 2 (4
    // bytes): `"` is at col 33 and `}` at 36.
    let source = "export declare class X { a: \"é—𝒳\"; }\n";
    let result = inject_dts_declarations_with_map(source, &[decl("X")], Some("qBAAA,YAAE,GAAC"));
    let decoded = decode(result.mappings.as_deref().unwrap());
    assert_eq!(decoded[1], vec![vec![21, 0, 0, 0], vec![33, 0, 0, 2]]);
    assert_eq!(decoded[4], vec![vec![0, 0, 0, 3]]);

    let identity = inject_dts_declarations_with_identity_map(source, &[decl("X")]);
    assert_identity(source, &identity.code, identity.mappings.as_deref().unwrap());
    let decoded = decode(identity.mappings.as_deref().unwrap());
    assert_eq!(lookup(&decoded, 4, 0), Some((0, 36)));
}

#[test]
fn keeps_a_trailing_import_comment_mapped() {
    let source = "import { Y } from \"y\"; // keep\nexport declare class X {\n  y: Y;\n}\n";
    let result = inject_dts_declarations_with_identity_map(source, &[decl("X")]);
    assert!(result.code.starts_with("import { Y } from \"y\"; // keep\nimport * as i0"));
    let decoded = decode(result.mappings.as_deref().unwrap());
    assert_eq!(lookup(&decoded, 0, 26), Some((0, 26)));
    assert_eq!(lookup(&decoded, 1, 0), None);
    assert_identity(source, &result.code, result.mappings.as_deref().unwrap());
}

#[test]
fn maps_every_token_of_several_classes() {
    let source = "/// <reference types=\"node\" />\nimport type { Y } from \"./y\";\n\
                  export declare class A {\n  a: Y;\n}\nexport declare class B {}\n\
                  export declare namespace N {\n    class C {\n    }\n}\n";
    let declarations = [decl("A"), decl("B"), decl("N.C")];
    let result = inject_dts_declarations_with_identity_map(source, &declarations);
    assert_eq!(result.code, inject_dts_declarations(source, &declarations));
    assert_eq!(result.code.matches("static ɵfac").count(), 3);
    assert_identity(source, &result.code, result.mappings.as_deref().unwrap());
}

#[test]
fn treats_carriage_return_as_a_column() {
    let source = "export declare class A {\r\n}\r\nexport declare class B {}\r\n";
    let declarations = [decl("A"), decl("B")];
    let result = inject_dts_declarations_with_identity_map(source, &declarations);
    assert_identity(source, &result.code, result.mappings.as_deref().unwrap());
    // `}` of B is at col 24, before `\r`; it moves to its own line.
    let decoded = decode(result.mappings.as_deref().unwrap());
    assert_eq!(result.code.split('\n').nth(8), Some("}\r"));
    assert_eq!(lookup(&decoded, 8, 0), Some((2, 24)));

    // A segment at that `\r` (col 25) moves with it, to col 1.
    let shifted = inject_dts_declarations_with_map(source, &declarations, Some(";;AAEA,yBAAyB"));
    let decoded = decode(shifted.mappings.as_deref().unwrap());
    assert_eq!(decoded[5], vec![vec![0, 0, 2, 0]]);
    assert_eq!(decoded[8], vec![vec![1, 0, 2, 25]]);
}

#[test]
fn drops_a_map_it_cannot_read() {
    let source = "export declare class X {\n}\n";
    let none = inject_dts_declarations_with_map(source, &[decl("X")], None);
    assert_eq!(none.mappings, None);
    let bad = inject_dts_declarations_with_map(source, &[decl("X")], Some("!!"));
    assert_eq!(bad.mappings, None);
    assert_eq!(bad.code, none.code);
    assert!(bad.code.contains("static ɵfac"));
    // A VLQ that never ends.
    let cut = inject_dts_declarations_with_map(source, &[decl("X")], Some("AAAg"));
    assert_eq!(cut.mappings, None);
}

#[test]
fn keeps_the_map_when_nothing_is_injected() {
    let source = "export declare class X {\n}\n";
    let missing = inject_dts_declarations_with_map(source, &[decl("Missing")], Some("AAAA;AACA"));
    assert_eq!(missing.code, source);
    assert_eq!(missing.mappings.as_deref(), Some("AAAA;AACA"));
    assert_eq!(
        inject_dts_declarations_with_identity_map(source, &[decl("Missing")]).mappings,
        None
    );

    // A second run injects nothing and keeps the shifted map.
    let once = inject_dts_declarations_with_map(source, &[decl("X")], Some("AAAA;AACA"));
    let twice =
        inject_dts_declarations_with_map(&once.code, &[decl("X")], once.mappings.as_deref());
    assert_eq!(twice.code, once.code);
    assert_eq!(twice.mappings, once.mappings);
}
