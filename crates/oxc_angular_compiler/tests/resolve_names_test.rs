//! Name resolution on long and deeply nested expressions.
//!
//! `resolve_names` runs from a post-order visitor over every expression. These
//! tests check that every name in a long chain still resolves to the right thing
//! (a local variable, the component context or `$event`) in bindings,
//! interpolations and listeners, inside and outside embedded views, and on many
//! sibling elements.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

/// Compiles `template` on a thread with a large stack: the parser and the
/// other phases recurse once per nesting level of these long chains.
fn compile(template: &str) -> String {
    let source = format!(
        "import {{ Component }} from '@angular/core';\n\
         @Component({{ selector: 'app-x', template: `{template}` }})\n\
         export class X {{}}\n"
    );
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let allocator = Allocator::default();
            let result = transform_angular_file(
                &allocator,
                "test.ts",
                &source,
                Some(&TransformOptions::default()),
                None,
            );
            assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
            // The emitter wraps long lines; drop all whitespace so chains compare as text.
            result.code.chars().filter(|c| !c.is_whitespace()).collect()
        })
        .unwrap()
        .join()
        .unwrap()
}

/// `a + b + c` as the emitter prints it, without whitespace: `((a+b)+c)`.
fn chain(terms: &[String]) -> String {
    let mut out = terms[0].clone();
    for term in &terms[1..] {
        out = format!("({out}+{term})");
    }
    out
}

fn repeat(terms: &[&str], n: usize) -> Vec<String> {
    (0..n).flat_map(|_| terms.iter().map(|t| (*t).to_string())).collect()
}

#[test]
fn long_chain_in_for_resolves_every_name() {
    let n = 40;
    let src_chain = repeat(&["item.a", "i", "x"], n).join(" + ");
    let code = compile(&format!(
        r#"@for (item of items; track item.id; let i = $index) {{ <p [title]="{src_chain}">{{{{ {src_chain} }}}}</p><button (click)="f(item, i, x, $event) + {src_chain}">b</button> }}"#
    ));

    let resolved = repeat(&["item_r3.a", "ɵ$index_1_r4", "ctx_r4.x"], n);
    let update = chain(&resolved);
    assert!(code.contains(&format!(r#"i0.ɵɵdomProperty("title",{update});"#)), "Got:\n{code}");
    assert!(code.contains(&format!("i0.ɵɵtextInterpolate({update});")), "Got:\n{code}");

    let mut listener = vec!["ctx_r4.f(item_r3,ɵ$index_1_r4,ctx_r4.x,$event)".to_string()];
    listener.extend(resolved);
    assert!(code.contains(&format!("returni0.ɵɵresetView({});", chain(&listener))), "Got:\n{code}");
}

#[test]
fn long_chain_in_root_listener_resolves_every_name() {
    let n = 60;
    let src_chain = repeat(&["a", "b"], n).join(" + ");
    let code = compile(&format!(
        r#"<i (click)="y = {src_chain}; g($event, {src_chain})">{{{{ {src_chain} }}}}</i>"#
    ));

    let resolved = chain(&repeat(&["ctx.a", "ctx.b"], n));
    assert!(code.contains(&format!("(ctx.y={resolved});")), "Got:\n{code}");
    assert!(code.contains(&format!("returnctx.g($event,{resolved});")), "Got:\n{code}");
    assert!(code.contains(&format!("i0.ɵɵtextInterpolate({resolved});")), "Got:\n{code}");
}

#[test]
fn many_siblings_resolve_names_in_order() {
    let n = 50;
    let template = "<i [title]=\"aK + b\" (click)=\"fK($event, aK + b)\"></i>";
    let template: String = (0..n).map(|k| template.replace('K', &k.to_string())).collect();
    let code = compile(&template);

    let mut last = 0;
    for k in 0..n {
        let listener = format!("returnctx.f{k}($event,(ctx.a{k}+ctx.b));");
        let pos = code.find(&listener).unwrap_or_else(|| panic!("missing {listener}:\n{code}"));
        assert!(pos >= last, "listener {k} out of order:\n{code}");
        last = pos;
    }
    let mut last = 0;
    for k in 0..n {
        let binding = format!(r#"i0.ɵɵdomProperty("title",(ctx.a{k}+ctx.b));"#);
        let pos = code.find(&binding).unwrap_or_else(|| panic!("missing {binding}:\n{code}"));
        assert!(pos >= last, "binding {k} out of order:\n{code}");
        last = pos;
    }
}
