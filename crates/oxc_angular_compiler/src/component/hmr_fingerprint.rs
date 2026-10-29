//! A fingerprint of a component file that ignores its inline template and
//! styles, for the HMR versus full-reload decision.
//!
//! A build tool keeps the fingerprint of each component file it compiled.
//! When the file changes, it takes the fingerprint again: if the two are
//! equal, the edit stayed inside the `template:` / `styles:` values of its
//! components, so an HMR update is enough. Anything else (the class body, an
//! import, another decorator field) changes the fingerprint, and the page
//! has to reload.
//!
//! The fingerprint covers the whole source text with the inside of those
//! values left out. Only the components the compiler compiled are looked at
//! (the caller names them), and only through Angular's own `@Component`, found
//! by its `@angular/core` import like the transform finds it. A value counts
//! only when it is one literal, so `'<p/>' + SUFFIX` or `STYLES` stay in the
//! fingerprint and an edit to them reloads.

use std::hash::{DefaultHasher, Hasher};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, Class, Expression, ObjectPropertyKind, Program, PropertyKey, PropertyKind,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

use super::decorator::find_component_decorator;
use crate::directive::{StringConsts, collect_string_consts};

/// The HMR fingerprint of a component file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComponentHmrFingerprint {
    /// Hash of the source with every excluded range left out. Two sources
    /// that differ only inside excluded ranges have the same fingerprint.
    pub fingerprint: String,
    /// The class names whose `@Component` was read, in source order.
    pub components: Vec<String>,
    /// The ranges left out of the fingerprint, in source order: the inside of
    /// each `template` / `styles` value, its quotes or brackets excluded.
    pub excluded: Vec<Span>,
}

/// Parse `source` and take its HMR fingerprint for the components named in
/// `class_names` (the ones the transform compiled).
///
/// A source with syntax errors excludes nothing: what the parser recovered
/// is not something to trust with an HMR decision, so any edit that leaves
/// it broken reloads the page.
pub fn component_hmr_fingerprint(
    allocator: &Allocator,
    path: &str,
    source: &str,
    class_names: &[&str],
) -> ComponentHmrFingerprint {
    if class_names.is_empty() {
        return fingerprint_with_exclusions(source, Vec::new(), Vec::new());
    }
    let source_type = SourceType::from_path(path).unwrap_or_default();
    let parser_ret = Parser::new(allocator, source, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return fingerprint_with_exclusions(source, Vec::new(), Vec::new());
    }
    let consts = collect_string_consts(allocator, &parser_ret.program);
    hmr_fingerprint_of_program(&parser_ret.program, source, &consts, |name| {
        class_names.contains(&name)
    })
}

/// [`component_hmr_fingerprint`] on an already parsed program. `consts` must
/// be collected from the same program; `is_compiled` says which class names
/// to read.
pub(super) fn hmr_fingerprint_of_program(
    program: &Program<'_>,
    source: &str,
    consts: &StringConsts<'_>,
    is_compiled: impl Fn(&str) -> bool,
) -> ComponentHmrFingerprint {
    let mut components = Vec::new();
    let mut excluded = Vec::new();
    // Every class the transform compiles, nested ones too. A name two classes
    // declare compiles without HMR, so neither is read, nor is a `namespace`
    // member.
    let declared_classes = crate::util::declared_classes(program);
    let ambiguous = super::transform::ambiguous_class_names(&declared_classes);
    for declared in &declared_classes {
        let class = declared.class;
        let Some(id) = &class.id else { continue };
        if declared.namespace.is_some()
            || ambiguous.contains(id.name.as_str())
            || !is_compiled(id.name.as_str())
        {
            continue;
        }
        if collect_excluded_ranges(class, consts, &mut excluded) {
            components.push(id.name.to_string());
        }
    }
    excluded.sort_unstable_by_key(|span| span.start);
    fingerprint_with_exclusions(source, components, excluded)
}

/// Push the inside of each literal `template` / `styles` value in `class`'s
/// `@Component({...})`. Returns whether the class has that decorator with an
/// object literal argument.
fn collect_excluded_ranges(
    class: &Class<'_>,
    consts: &StringConsts<'_>,
    excluded: &mut Vec<Span>,
) -> bool {
    let Some(decorator) = find_component_decorator(&class.decorators, consts) else {
        return false;
    };
    let Expression::CallExpression(call) = &decorator.expression else { return false };
    let Some(Argument::ObjectExpression(config)) = call.arguments.first() else {
        return false;
    };
    for property in &config.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else { continue };
        // `get template() {}`, `styles() {}` and `{ styles }` name the field
        // but hold no literal; a computed key's name is not read.
        if property.kind != PropertyKind::Init
            || property.method
            || property.shorthand
            || property.computed
        {
            continue;
        }
        let name = match &property.key {
            PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
            PropertyKey::StringLiteral(lit) => lit.value.as_str(),
            _ => None,
        };
        // `template` is a string; `styles` is a string or an array of them.
        let literal = match (name, &property.value) {
            (
                Some("template" | "styles"),
                Expression::StringLiteral(_) | Expression::TemplateLiteral(_),
            )
            | (Some("styles"), Expression::ArrayExpression(_)) => property.value.span(),
            _ => continue,
        };
        // Keep the quotes or brackets: `styles: []` and `styles: ''` stay
        // apart, and the value's shape is part of the fingerprint.
        if literal.end > literal.start + 2 {
            excluded.push(Span::new(literal.start + 1, literal.end - 1));
        }
    }
    true
}

/// Hash `source` without `excluded` (sorted, non-overlapping).
fn fingerprint_with_exclusions(
    source: &str,
    components: Vec<String>,
    excluded: Vec<Span>,
) -> ComponentHmrFingerprint {
    // Two independently seeded 64-bit SipHashes plus the kept length. Both
    // fingerprints of a comparison come from the same build, so the std
    // hasher's fixed keys are enough; it is not meant to resist an attacker.
    let mut first = DefaultHasher::new();
    let mut second = DefaultHasher::new();
    second.write_u64(0x9e37_79b9_7f4a_7c15);
    let bytes = source.as_bytes();
    let mut kept = 0usize;
    let mut cursor = 0usize;
    let mut write = |segment: &[u8]| {
        first.write(segment);
        second.write(segment);
        kept += segment.len();
    };
    for span in &excluded {
        write(&bytes[cursor..span.start as usize]);
        cursor = span.end as usize;
    }
    write(&bytes[cursor..]);
    ComponentHmrFingerprint {
        fingerprint: format!("{:016x}{:016x}-{kept:x}", first.finish(), second.finish()),
        components,
        excluded,
    }
}
