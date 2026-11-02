//! What ngtsc's `enableTemplateSourceLocations` and `rootDirs` options need:
//! the project-relative path of a file, and where a template's text sits in
//! the source its positions are counted in.

use oxc_allocator::Allocator;
use oxc_str::Ident;

use super::metadata::{AngularVersion, ComponentMetadata, Prerelease};
use super::transform::ResolvedResources;
use crate::pipeline::phases::attach_source_locations::{TemplateSource, utf16_len};

/// Whether the runtime has `ɵɵattachSourceLocations`: it first shipped in
/// `19.1.0-next.3` (angular/angular#58982). An unknown version is the latest.
pub(super) fn supports_attach_source_locations(version: Option<AngularVersion>) -> bool {
    version.is_none_or(|v| match (v.major, v.minor, v.patch) {
        (19, 1, 0) => v.prerelease.is_none_or(|p| p >= Prerelease::Next(3)),
        triple => triple > (19, 1, 0),
    })
}

/// `file` relative to the first of `root_dirs` that contains it, or `None`
/// when none does.
///
/// Mirrors ngtsc's `getProjectRelativePath` (compiler-cli `util/src/path.ts`),
/// with root dirs resolved against the working directory like `getRootDirs`.
/// Unlike ngtsc, the path is never lowercased: ngtsc's `getCanonicalFileName`
/// lowercases it on a case-insensitive file system (macOS, Windows), so the
/// two only match there for lowercase paths.
pub(super) fn project_relative_path(file: &str, root_dirs: &[String]) -> Option<String> {
    let file = resolve(file);
    root_dirs.iter().find_map(|root| {
        let rel = relative(&resolve(root), &file);
        (!rel.starts_with("..")).then_some(rel)
    })
}

/// The path segments of `path` made absolute against the working directory,
/// with `.` and `..` resolved and `\` read as `/`.
fn resolve(path: &str) -> Vec<String> {
    let path = path.replace('\\', "/");
    let bytes = path.as_bytes();
    let is_absolute = path.starts_with('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    let full = if is_absolute {
        path
    } else {
        // Without a working directory (wasm), a relative path stays relative:
        // two such paths still compare against the same unknown base.
        match std::env::current_dir() {
            Ok(cwd) => format!("{}/{path}", cwd.to_string_lossy().replace('\\', "/")),
            Err(_) => path,
        }
    };
    let mut segments: Vec<String> = Vec::new();
    for segment in full.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment.to_string()),
        }
    }
    segments
}

/// Node's `path.relative(from, to)` on resolved segments.
fn relative(from: &[String], to: &[String]) -> String {
    let common = from.iter().zip(to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(to[common..].iter().map(String::as_str));
    parts.join("/")
}

/// The template path and source text for `ɵɵattachSourceLocations`, the way
/// ngtsc's `ComponentDecoratorHandler.analyze` and `extractTemplate` give them:
///
/// - a `templateUrl` template: the `.html` file, positions from its start;
/// - an inline string or no-substitution template literal: the `.ts` file,
///   positions in its raw text;
/// - any other inline template (a variable, a concatenation): the `.ts`
///   file, positions in the template value from 0.
///
/// `None` when there is nothing to emit: no root dirs (ngtsc's root is
/// `rootDir`, else the working directory; oxc gets no root here and emits
/// nothing, like ngtsc for a file outside its root), a file outside the root
/// dirs, a `templateUrl` that is not relative to the component file, or an
/// inline literal whose raw text oxc cannot map to its value.
pub(super) fn template_source<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'_>,
    template: &'a str,
    file_path: &str,
    source: &str,
    root_dirs: Option<&[String]>,
    resolved_resources: Option<&ResolvedResources>,
) -> Option<(Ident<'a>, TemplateSource<'a>)> {
    let root_dirs = root_dirs?;
    let whole =
        |text| TemplateSource { text, start: 0, start_line: 0, start_column: 0, raw_offsets: None };

    // `templateResource.path` (the resolved `templateUrl`) when the template
    // is an external file, as `TemplateOrigin::of_component` decides it.
    if let Some(url) = &metadata.template_url
        && resolved_resources.is_some()
    {
        let url = url.as_str().replace('\\', "/");
        if url.starts_with('/') {
            // ngtsc looks a rooted URL up in each root dir; oxc cannot tell which.
            return None;
        }
        let file = file_path.replace('\\', "/");
        let dir = file.rfind('/').map_or("", |i| &file[..i]);
        let path = project_relative_path(&format!("{dir}/{url}"), root_dirs)?;
        return Some((Ident::from(allocator.alloc_str(&path)), whole(template)));
    }

    if metadata.template.as_ref().is_none_or(|inline| inline.as_str() != template) {
        return None;
    }
    let path = Ident::from(allocator.alloc_str(&project_relative_path(file_path, root_dirs)?));
    let Some(span) = metadata.template_span else {
        return Some((path, whole(template)));
    };
    let raw = source.get(span.start as usize..span.end as usize)?;
    let raw_offsets = if raw == template {
        None
    } else {
        let is_template_literal = source.as_bytes().get(span.start as usize - 1) == Some(&b'`');
        let (cooked, offsets) = cook_literal(raw, span.start, is_template_literal)?;
        if cooked != template {
            return None;
        }
        Some(&*allocator.alloc_slice_copy(&offsets))
    };
    let (start_line, start_column) = typescript_line_and_character(source, span.start as usize);
    Some((
        path,
        TemplateSource {
            text: allocator.alloc_str(source),
            start: span.start,
            start_line,
            start_column,
            raw_offsets,
        },
    ))
}

/// The 0-based line and UTF-16 character of byte `offset`, as TypeScript's
/// `getLineAndCharacterOfPosition` gives them: `\r\n`, `\r`, `\n`, U+2028 and
/// U+2029 each end a line.
fn typescript_line_and_character(source: &str, offset: usize) -> (u32, u32) {
    let (mut line, mut character) = (0, 0);
    let mut chars = source[..offset.min(source.len())].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                line += 1;
                character = 0;
            }
            _ => character += utf16_len(c),
        }
    }
    (line, character)
}

/// The value of a string or template literal from its raw text (between the
/// quotes), with the byte offset in the source of the character each value
/// byte comes from: an escaped character comes from its backslash, as in
/// Angular's `EscapedCharacterCursor`.
///
/// `None` for escapes oxc does not map (legacy octal, `\u` surrogates); the
/// caller then emits no source locations.
fn cook_literal(raw: &str, base: u32, is_template_literal: bool) -> Option<(String, Vec<u32>)> {
    let mut cooked = String::with_capacity(raw.len());
    let mut offsets = Vec::with_capacity(raw.len());
    let mut push = |c: char, at: usize| {
        cooked.push(c);
        let at = base.saturating_add(u32::try_from(at).unwrap_or(u32::MAX));
        offsets.extend(std::iter::repeat_n(at, c.len_utf8()));
    };
    let mut chars = raw.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' => {
                let (_, escape) = chars.next()?;
                let value = match escape {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'v' => '\u{b}',
                    '0' if !chars.peek().is_some_and(|(_, c)| c.is_ascii_digit()) => '\0',
                    '0'..='9' => return None,
                    'x' => {
                        let hex: String =
                            (0..2).filter_map(|_| chars.next().map(|(_, c)| c)).collect();
                        char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
                    }
                    'u' => {
                        let hex: String = if chars.peek().is_some_and(|&(_, c)| c == '{') {
                            chars.next();
                            chars.by_ref().map(|(_, c)| c).take_while(|&c| c != '}').collect()
                        } else {
                            (0..4).filter_map(|_| chars.next().map(|(_, c)| c)).collect()
                        };
                        // A surrogate (half of a pair) has no `char`.
                        char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
                    }
                    // A line continuation adds nothing.
                    '\r' => {
                        chars.next_if(|&(_, c)| c == '\n');
                        continue;
                    }
                    '\n' | '\u{2028}' | '\u{2029}' => continue,
                    other => other,
                };
                push(value, at);
            }
            // A template literal's value reads `\r\n` and `\r` as `\n`.
            '\r' if is_template_literal => {
                chars.next_if(|&(_, c)| c == '\n');
                push('\n', at);
            }
            _ => push(c, at),
        }
    }
    Some((cooked, offsets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_to_the_first_root_that_contains_the_file() {
        let roots = ["/w/lib".to_string(), "/w".to_string()];
        assert_eq!(project_relative_path("/w/src/a.ts", &roots).as_deref(), Some("src/a.ts"));
        assert_eq!(project_relative_path("/w/lib/b.ts", &roots).as_deref(), Some("b.ts"));
        assert_eq!(project_relative_path("/x/a.ts", &roots), None);
        assert_eq!(project_relative_path("/w/./src/../a.ts", &roots).as_deref(), Some("a.ts"));
        assert_eq!(
            project_relative_path("C:\\w\\src\\a.ts", &["C:/w".to_string()]).as_deref(),
            Some("src/a.ts")
        );
    }

    #[test]
    fn cooks_escapes_to_their_backslash() {
        let (cooked, offsets) = cook_literal(
            r"a\nb\x3Cc\u{41}\
d",
            10,
            false,
        )
        .unwrap();
        assert_eq!(cooked, "a\nb<cAd");
        assert_eq!(offsets, vec![10, 11, 13, 14, 18, 19, 27]);
        assert_eq!(cook_literal(r"\uD83D\uDE00", 0, false), None);
        assert_eq!(cook_literal("a\r\nb", 0, true).unwrap().0, "a\nb");
    }

    #[test]
    fn version_gate() {
        assert!(supports_attach_source_locations(None));
        assert!(!supports_attach_source_locations(Some(AngularVersion::new(19, 0, 5))));
        assert!(supports_attach_source_locations(Some(AngularVersion::new(19, 1, 0))));
        let next = |n| Some(AngularVersion::new(19, 1, 0).with_prerelease(Prerelease::Next(n)));
        assert!(!supports_attach_source_locations(next(2)));
        assert!(supports_attach_source_locations(next(3)));
    }
}
