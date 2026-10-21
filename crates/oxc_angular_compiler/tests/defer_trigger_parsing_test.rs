//! `@defer` trigger parsing (`on`, `prefetch on`, `hydrate on`, `when`, ...)
//! matches Angular's `r3_deferred_triggers.ts` / `r3_deferred_blocks.ts`.
//!
//! `fixtures/defer_triggers_ngtsc.json` is a snapshot of what `parseTemplate`
//! from `@angular/compiler` (version in the file) reports for each template:
//! the errors (message and span) and the parsed triggers of each kind. Each
//! case is summarized the same way on both sides, see `summarize`. Expression
//! parser errors are compared by kind only, since their text carries
//! positions formatted differently. Cases whose difference lies outside
//! trigger parsing carry a `skip` reason.

use oxc_allocator::Allocator;
use oxc_angular_compiler::ast::expression::{AngularExpression, LiteralMapKey};
use oxc_angular_compiler::ast::r3::{R3DeferredBlockTriggers, R3Node};
use oxc_angular_compiler::parser::html::HtmlParser;
use oxc_angular_compiler::transform::html_to_r3::{HtmlToR3Transform, TransformOptions};
use oxc_span::Span;
use serde_json::Value;

const FIXTURES: &str = include_str!("fixtures/defer_triggers_ngtsc.json");

fn span(span: Span) -> String {
    format!("{}..{}", span.start, span.end)
}

fn opt_span(s: Option<Span>) -> String {
    s.map_or_else(|| "-".to_string(), span)
}

fn time(value: f64) -> String {
    value.to_string()
}

fn expression_class(expr: &AngularExpression<'_>) -> &'static str {
    match expr {
        AngularExpression::Empty(_) => "EmptyExpr",
        AngularExpression::PropertyRead(_) => "PropertyRead",
        AngularExpression::Binary(_) => "Binary",
        AngularExpression::Call(_) => "Call",
        AngularExpression::LiteralPrimitive(_) => "LiteralPrimitive",
        _ => "Other",
    }
}

/// One trigger set, in the format of the fixture's `on:`/`prefetch:`/`hydrate:` lines.
fn summarize_triggers(t: &R3DeferredBlockTriggers<'_>) -> String {
    let reference = |r: &Option<oxc_str::Ident<'_>>| {
        r.as_ref().map_or_else(|| "-".to_string(), std::string::ToString::to_string)
    };
    let mut out = Vec::new();
    if let Some(w) = &t.when {
        out.push(format!(
            "when(value={} span={})",
            expression_class(&w.value),
            span(w.source_span)
        ));
    }
    if let Some(i) = &t.idle {
        out.push(format!(
            "idle(timeout={} span={} name={})",
            i.timeout.map_or_else(|| "-".to_string(), time),
            span(i.source_span),
            opt_span(i.name_span)
        ));
    }
    if let Some(i) = &t.immediate {
        out.push(format!("immediate(span={} name={})", span(i.source_span), opt_span(i.name_span)));
    }
    if let Some(h) = &t.hover {
        out.push(format!(
            "hover(ref={} span={} name={})",
            reference(&h.reference),
            span(h.source_span),
            span(h.name_span)
        ));
    }
    if let Some(tm) = &t.timer {
        out.push(format!(
            "timer(delay={} span={} name={})",
            time(tm.delay),
            span(tm.source_span),
            span(tm.name_span)
        ));
    }
    if let Some(i) = &t.interaction {
        out.push(format!(
            "interaction(ref={} span={} name={})",
            reference(&i.reference),
            span(i.source_span),
            span(i.name_span)
        ));
    }
    if let Some(v) = &t.viewport {
        let options = match &v.options {
            Some(AngularExpression::LiteralMap(map)) => {
                let keys: Vec<&str> = map
                    .keys
                    .iter()
                    .map(|k| match k {
                        LiteralMapKey::Property(p) => p.key.as_str(),
                        LiteralMapKey::Spread(_) => "...",
                    })
                    .collect();
                format!("{{{}}}", keys.join(","))
            }
            Some(_) => "?".to_string(),
            None => "-".to_string(),
        };
        out.push(format!(
            "viewport(ref={} options={options} span={} name={})",
            reference(&v.reference),
            span(v.source_span),
            span(v.name_span)
        ));
    }
    if let Some(n) = &t.never {
        out.push(format!("never(span={} name={})", span(n.source_span), opt_span(n.name_span)));
    }
    out.join(" ")
}

/// Errors and triggers of the first `@defer` block in `template`, one line each.
fn summarize(template: &str) -> Vec<String> {
    let allocator = Allocator::default();
    let html_result = HtmlParser::new(&allocator, template, "test.html").parse();
    let mut lines: Vec<String> = html_result
        .errors
        .iter()
        .map(|e| format!("error {}..{} {}", e.span.start.offset, e.span.end.offset, e.msg))
        .collect();

    let transformer = HtmlToR3Transform::new(&allocator, template, TransformOptions::default());
    let r3_result = transformer.transform(&html_result.nodes);
    for e in &r3_result.errors {
        // Expression parser errors don't carry a span here.
        if e.span.start.offset == 0 && e.span.end.offset == 0 {
            lines.push("error Parser Error".to_string());
        } else {
            lines.push(format!("error {}..{} {}", e.span.start.offset, e.span.end.offset, e.msg));
        }
    }

    if let Some(R3Node::DeferredBlock(block)) =
        r3_result.nodes.iter().find(|n| matches!(n, R3Node::DeferredBlock(_)))
    {
        lines.push(format!("on: {}", summarize_triggers(&block.triggers)));
        lines.push(format!("prefetch: {}", summarize_triggers(&block.prefetch_triggers)));
        lines.push(format!("hydrate: {}", summarize_triggers(&block.hydrate_triggers)));
    }
    lines
}

#[test]
fn defer_triggers_match_ngtsc() {
    let fixtures: Value = serde_json::from_str(FIXTURES).unwrap();
    let mut failures = Vec::new();
    for case in fixtures["cases"].as_array().unwrap() {
        if case.get("skip").is_some() {
            continue;
        }
        let template = case["template"].as_str().unwrap();
        let expected: Vec<String> = case["summary"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_string())
            .collect();
        let actual = summarize(template);
        if actual != expected {
            failures
                .push(format!("{template:?}\n  expected: {expected:#?}\n  actual:   {actual:#?}"));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}
