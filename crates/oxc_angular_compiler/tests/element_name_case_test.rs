//! Element names are case-sensitive and kept as written. `<View>` is an element named
//! "View": whether a component matches it is decided later, by selector.
//!
//! An element whose name starts with a capital letter used to be taken for a
//! selectorless component, a node kind that only the selectorless lexer mode produces
//! and that ingest discards, so `<View><text>hi</text></View>` compiled to an empty
//! template with no error.
//!
//! The expected output is ngtsc 22.2.2's for the same template (the default is
//! the latest release line), in this emitter's spelling. The components are
//! declared `standalone: false` so their templates use the full instructions
//! (`ɵɵelementStart`) rather than the DOM-only ones a standalone component
//! without dependencies gets.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    AngularVersion, TransformOptions,
    output::emitter::JsEmitter,
    parser::html::HtmlParser,
    pipeline::{emit::compile_template, ingest::ingest_component},
    transform::html_to_r3::{HtmlToR3Transform, TransformOptions as HtmlTransformOptions},
    transform_angular_file,
};
use oxc_str::Ident;

/// The whitespace-free definition of the component compiled from `template`.
fn definition(template: &str) -> String {
    definition_for(template, None)
}

/// `definition` pinned to an Angular version (`None` is the latest line).
fn definition_for(template: &str, angular_version: Option<AngularVersion>) -> String {
    let source = format!(
        "import {{ Component }} from '@angular/core';\n\
         @Component({{ selector: 'x', standalone: false, template: `{template}` }})\n\
         export class X {{ t = ''; on = true; items: any[] = []; go(..._a: any[]) {{}} }}\n"
    );
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "/x/a.ts",
        &source,
        Some(&TransformOptions { angular_version, ..TransformOptions::default() }),
        None,
    );
    assert!(result.diagnostics.is_empty(), "`{template}`: {:?}", result.diagnostics);
    let compact: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
    let start = compact.find("i0.ɵɵdefineComponent(").expect("definition");
    let end = compact[start..].find("(()=>{").map_or(compact.len(), |i| start + i);
    // Embedded view functions are declared before the class.
    let declarations = compact[..start].find("functionX_").map_or("", |i| &compact[i..start]);
    let declarations = declarations.split("exportclassX{").next().unwrap_or_default();
    format!("{declarations}{}", &compact[start..end])
}

#[track_caller]
fn assert_compiles_to(template: &str, expected: &[&str]) {
    let definition = definition(template);
    for part in expected {
        assert!(definition.contains(part), "`{template}` should contain `{part}`:\n{definition}");
    }
}

/// The reported template.
#[test]
fn capitalised_root_element_with_a_lowercase_child() {
    assert_compiles_to(
        "<View><text>hi</text></View>",
        &[
            "decls:3,vars:0,",
            r#"i0.ɵɵelementStart(0,"View")(1,"text");i0.ɵɵtext(2,"hi");i0.ɵɵelementEnd()();"#,
        ],
    );
}

#[test]
fn capitalised_element_nested_in_a_lowercase_one() {
    assert_compiles_to(
        "<div><View><span>a</span><Text>b</Text></View></div>",
        &[
            "decls:6,vars:0,",
            r#"i0.ɵɵelementStart(0,"div")(1,"View")(2,"span");i0.ɵɵtext(3,"a");i0.ɵɵelementEnd();i0.ɵɵelementStart(4,"Text");i0.ɵɵtext(5,"b");"#,
        ],
    );
}

#[test]
fn mixed_case_names_are_kept_as_written() {
    assert_compiles_to(
        "<myWidget>a</myWidget><My-Widget>b</My-Widget><X>c</X>",
        &[
            "decls:6,vars:0,",
            r#"i0.ɵɵelementStart(0,"myWidget");"#,
            r#"i0.ɵɵelementStart(2,"My-Widget");"#,
            r#"i0.ɵɵelementStart(4,"X");"#,
        ],
    );
}

#[test]
fn capitalised_element_with_attributes_bindings_listeners_and_a_reference() {
    assert_compiles_to(
        r#"<View class="a" id="b" [title]="t" [class.x]="on" (press)="go(ref)" #ref>{{ t }}</View>"#,
        &[
            "decls:3,vars:4,",
            r#"consts:[["ref",""],["id","b",1,"a",3,"press","title"]]"#,
            r#"i0.ɵɵelementStart(0,"View",1,0);"#,
            r#"i0.ɵɵlistener("press",functionX_Template_View_press_0_listener(){"#,
            r#"i0.ɵɵclassProp("x",ctx.on);i0.ɵɵproperty("title",ctx.t);"#,
        ],
    );
}

#[test]
fn self_closing_capitalised_elements() {
    assert_compiles_to(
        r#"<View /><Row [title]="t" /><div><Cell /></div>"#,
        &[
            "decls:4,vars:1,",
            r#"i0.ɵɵelement(0,"View")(1,"Row",0);i0.ɵɵelementStart(2,"div");i0.ɵɵelement(3,"Cell");i0.ɵɵelementEnd();"#,
        ],
    );
}

#[test]
fn capitalised_elements_inside_control_flow_and_templates() {
    assert_compiles_to(
        "@if (on) { <View>a</View> } @for (i of items; track i) { <Row>{{ i }}</Row> } \
         <ng-template><Cell>c</Cell></ng-template><Star *ngIf=\"on\">d</Star>",
        &[
            r#"functionX_Conditional_0_Template(rf,ctx){if((rf&1)){i0.ɵɵelementStart(0,"View");i0.ɵɵtext(1,"a");i0.ɵɵelementEnd();}}"#,
            r#"functionX_For_2_Template(rf,ctx){if((rf&1)){i0.ɵɵelementStart(0,"Row");i0.ɵɵtext(1);i0.ɵɵelementEnd();}"#,
            r#"functionX_ng_template_3_Template(rf,ctx){if((rf&1)){i0.ɵɵelementStart(0,"Cell");i0.ɵɵtext(1,"c");i0.ɵɵelementEnd();}}"#,
            r#"functionX_Star_4_Template(rf,ctx){if((rf&1)){i0.ɵɵelementStart(0,"Star");i0.ɵɵtext(1,"d");i0.ɵɵelementEnd();}}"#,
            r#"i0.ɵɵconditionalCreate(0,X_Conditional_0_Template,2,0,"View");"#,
            r#"i0.ɵɵrepeaterCreate(1,X_For_2_Template,2,1,"Row",null,i0.ɵɵrepeaterTrackByIdentity);"#,
            r#"i0.ɵɵtemplate(3,X_ng_template_3_Template,2,0,"ng-template")(4,X_Star_4_Template,2,0,"Star",0);"#,
        ],
    );
}

/// A known HTML element keeps its spelling but still behaves like the element it names:
/// void, optional end tag, security context.
#[test]
fn html_elements_written_in_capitals() {
    assert_compiles_to(
        r#"<DIV class="a">a</DIV><BUTTON (click)="go()">c</BUTTON>"#,
        &[r#"i0.ɵɵelementStart(0,"DIV",0);"#, r#"i0.ɵɵelementStart(2,"BUTTON",1);"#],
    );
    assert_compiles_to(
        "<UL><LI>d<LI>e</UL><P>p<P>q",
        &[
            "decls:9,vars:0,",
            r#"i0.ɵɵelementStart(0,"UL")(1,"LI");i0.ɵɵtext(2,"d");i0.ɵɵelementEnd();i0.ɵɵelementStart(3,"LI");i0.ɵɵtext(4,"e");i0.ɵɵelementEnd()();"#,
            r#"i0.ɵɵelementStart(5,"P");i0.ɵɵtext(6,"p");i0.ɵɵelementEnd();i0.ɵɵelementStart(7,"P");"#,
        ],
    );
    let img_src = definition(r#"<INPUT [value]="t"><BR><Img [src]="t">after"#);
    assert!(
        img_src.contains(r#"i0.ɵɵelement(0,"INPUT",0)(1,"BR")(2,"Img",1);i0.ɵɵtext(3,"after");"#),
        "{img_src}"
    );
    // 22.2.2 (`angular/angular@439ff1bdf4`) removed `img|src` from the URL
    // security context, so bound `src` no longer receives `ɵɵsanitizeUrl`.
    assert!(img_src.contains(r#"i0.ɵɵproperty("src",ctx.t)"#), "{img_src}");
    assert!(!img_src.contains("sanitizeUrl"), "{img_src}");
    // Versions through 22.2.1 still sanitize it.
    let img_src_22_2_1 = definition_for(r#"<Img [src]="t">"#, Some(AngularVersion::new(22, 2, 1)));
    assert!(
        img_src_22_2_1.contains(r#"i0.ɵɵproperty("src",ctx.t,i0.ɵɵsanitizeUrl)"#),
        "{img_src_22_2_1}"
    );
    assert_compiles_to(
        r#"<A [href]="t">a</A><IFRAME [src]="t"></IFRAME>"#,
        &[
            r#"i0.ɵɵproperty("href",ctx.t,i0.ɵɵsanitizeUrl);"#,
            r#"i0.ɵɵproperty("src",ctx.t,i0.ɵɵsanitizeResourceUrl);"#,
        ],
    );
}

/// `<STYLE>` and `<SCRIPT>` are removed from the template like `<style>` and `<script>`,
/// and `<TEXTAREA>` and `<TITLE>` keep their text.
#[test]
fn raw_text_elements_written_in_capitals() {
    assert_compiles_to(
        "<STYLE>p{}</STYLE><Script>x()</Script><p>a</p>",
        &[
            "decls:2,vars:0,",
            r#"i0.ɵɵelementStart(0,"p");i0.ɵɵtext(1,"a");i0.ɵɵelementEnd();"#,
            r#"styles:["p[_ngcontent-%COMP%]{}"]"#,
        ],
    );
    assert_compiles_to(
        "<TEXTAREA>{{ t }}</TEXTAREA><Title>{{ t }}</Title>",
        &[
            "decls:4,vars:2,",
            r#"i0.ɵɵelementStart(0,"TEXTAREA");i0.ɵɵtext(1);i0.ɵɵelementEnd();i0.ɵɵelementStart(2,"Title");"#,
        ],
    );
}

/// The preparser lowercases the name before it looks for `ng-content`, so any spelling
/// projects content. `ng-container` and `ng-template` are matched as written, so in
/// capitals they are ordinary elements.
#[test]
fn angular_element_names_written_in_capitals() {
    assert_compiles_to(
        r#"<NG-CONTENT select="[a]" /><Ng-Content />"#,
        &[
            "ngContentSelectors:_c1,decls:2,vars:0,",
            "i0.ɵɵprojectionDef(_c0);i0.ɵɵprojection(0);i0.ɵɵprojection(1,1);",
        ],
    );
    assert_compiles_to(
        "<NG-CONTAINER>a</NG-CONTAINER><Ng-Template>b</Ng-Template>",
        &[
            "decls:4,vars:0,",
            r#"i0.ɵɵelementStart(0,"NG-CONTAINER");i0.ɵɵtext(1,"a");i0.ɵɵelementEnd();i0.ɵɵelementStart(2,"Ng-Template");"#,
        ],
    );
}

/// Names inside `<svg>` are legitimately mixed case, and a capitalised `<SVG>` still
/// puts its children in the svg namespace.
#[test]
fn namespaced_elements() {
    assert_compiles_to(
        r#"<svg><Circle r="1" /><foreignObject><DIV>x</DIV></foreignObject><linearGradient /></svg><svg:Circle />"#,
        &[
            r#"i0.ɵɵnamespaceSVG();i0.ɵɵelementStart(0,"svg");i0.ɵɵelement(1,"Circle",0);i0.ɵɵelementStart(2,"foreignObject");i0.ɵɵnamespaceHTML();i0.ɵɵelementStart(3,"DIV");"#,
            r#"i0.ɵɵnamespaceSVG();i0.ɵɵelement(5,"linearGradient");i0.ɵɵelementEnd();i0.ɵɵelement(6,"Circle");"#,
        ],
    );
    assert_compiles_to(
        "<SVG><rect /></SVG><MATH><mi>x</mi></MATH>",
        &[
            r#"i0.ɵɵnamespaceSVG();i0.ɵɵelementStart(0,"SVG");i0.ɵɵelement(1,"rect");i0.ɵɵelementEnd();"#,
            r#"i0.ɵɵnamespaceMathML();i0.ɵɵelementStart(2,"MATH")(3,"mi");"#,
        ],
    );
}

#[test]
fn lowercase_elements_are_unchanged() {
    assert_compiles_to(
        r#"<view><text>hi</text></view><my-widget [title]="t" (press)="go()">a</my-widget>"#,
        &[
            "decls:5,vars:1,",
            r#"i0.ɵɵelementStart(0,"view")(1,"text");i0.ɵɵtext(2,"hi");i0.ɵɵelementEnd()();i0.ɵɵelementStart(3,"my-widget",0);"#,
        ],
    );
}

/// Only a selectorless parse produces component nodes. The pipeline cannot compile
/// them, and says so rather than emitting a template without them.
#[test]
fn selectorless_component_node_is_reported() {
    let template = "<p>a</p><MyComp>b</MyComp>";
    let allocator = Allocator::default();
    let html = HtmlParser::with_selectorless(&allocator, template, "test.html").parse();
    assert!(html.errors.is_empty(), "{:?}", html.errors);
    let r3 = HtmlToR3Transform::new(&allocator, template, HtmlTransformOptions::default())
        .transform(&html.nodes);
    assert!(r3.errors.is_empty(), "{:?}", r3.errors);
    let mut job = ingest_component(&allocator, Ident::from("X"), r3.nodes);
    let result = compile_template(&mut job);

    let messages: Vec<&str> = job.diagnostics.iter().map(|d| &*d.message).collect();
    assert_eq!(
        messages,
        [
            "Selectorless component <MyComp> cannot be compiled: selectorless components are not supported."
        ]
    );
    // The rest of the template is still compiled.
    let emitted = JsEmitter::new().emit_statements(&result.template_fn.statements);
    assert!(emitted.contains(r#"i0.ɵɵelementStart(0,"p")"#), "{emitted}");
}

/// A template stylesheet `<link>` compiles its resolvable URL into the
/// component's styles and disappears from the emitted template, like the
/// preparser classifying it STYLESHEET and the transform returning `null`.
/// Attribute names are matched case-insensitively and in any order, because
/// `preparseElement` lowercases them.
#[test]
fn capitalised_stylesheet_links() {
    use std::collections::HashMap;

    use oxc_angular_compiler::ResolvedResources;

    let cases = [
        // Uppercase tag and attributes, `HREF` before `REL`.
        "<LINK HREF=\"./theme.css\" REL=\"stylesheet\" />",
        // Mixed case tag, lowercase attributes.
        "<Link rel=\"stylesheet\" href=\"./theme.css\">",
    ];
    for template in cases {
        let source = format!(
            "import {{ Component }} from '@angular/core';\n\
             @Component({{ selector: 'x', template: `{template}` }})\n\
             export class X {{}}"
        );
        let allocator = Allocator::default();
        let mut styles = HashMap::new();
        styles.insert("./theme.css".to_string(), vec![".link-style{color:red}".to_string()]);
        let resources = ResolvedResources { styles, ..Default::default() };
        let result = transform_angular_file(
            &allocator,
            "/x/a.ts",
            &source,
            Some(&TransformOptions::default()),
            Some(&resources),
        );
        assert!(result.diagnostics.is_empty(), "`{template}`: {:?}", result.diagnostics);
        // The collected stylesheet joins the component's styles…
        assert!(
            result.code.contains("link-style"),
            "`{template}` should compile the linked stylesheet:\n{}",
            result.code
        );
        // …and the link element is dropped from the template.
        let compact: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            !compact.contains("LINK") || !compact.contains("ɵɵelement"),
            "`{template}` should not emit a link element:\n{compact}"
        );
    }
}

/// A resolvable stylesheet `<link>` missing from the provided resources is a
/// COMPONENT_RESOURCE_NOT_FOUND diagnostic like a missing `styleUrl`.
#[test]
fn missing_stylesheet_link_is_a_resource_error() {
    use std::collections::HashMap;

    use oxc_angular_compiler::ResolvedResources;

    let source = "import { Component } from '@angular/core';\n\
                  @Component({ selector: 'x', template: `<LINK rel=\"stylesheet\" href=\"./theme.css\">` })\n\
                  export class X {}";
    let allocator = Allocator::default();
    let resources = ResolvedResources { styles: HashMap::new(), ..Default::default() };
    let result = transform_angular_file(
        &allocator,
        "/x/a.ts",
        source,
        Some(&TransformOptions::default()),
        Some(&resources),
    );
    let messages: Vec<String> = result.diagnostics.iter().map(|d| d.message.to_string()).collect();
    assert!(
        messages.iter().any(|m| m.contains("style URL './theme.css' could not be resolved")),
        "{messages:?}"
    );
}

/// Inside `ngNonBindable` a stylesheet `<link>` is still dropped
/// (`NonBindableVisitor` checks `type === STYLESHEET`), but its URL is not
/// recorded — and an unresolvable or missing href does not save it, because
/// `isStyleUrlResolvable` is not consulted there. Outside `ngNonBindable` an
/// unresolvable stylesheet link stays an ordinary element.
#[test]
fn stylesheet_links_inside_ng_non_bindable() {
    let non_bindable_div = r#"i0.ɵɵelementStart(0,"div");i0.ɵɵdisableBindings();i0.ɵɵtext(1,"a");i0.ɵɵenableBindings();"#;
    assert_compiles_to(
        r#"<div ngNonBindable><Link rel="stylesheet" href="./theme.css">a</div>"#,
        &[non_bindable_div],
    );
    assert_compiles_to(
        r#"<div ngNonBindable><LINK REL="stylesheet" HREF="https://x/t.css">a</div>"#,
        &[non_bindable_div],
    );
    assert_compiles_to(r#"<div ngNonBindable><link rel="stylesheet">a</div>"#, &[non_bindable_div]);
    // Outside ngNonBindable an unresolvable stylesheet link stays an element,
    // and so does a link without `rel="stylesheet"`.
    assert_compiles_to(
        r#"<link rel="stylesheet" href="https://x/t.css">"#,
        &[r#"i0.ɵɵelement(0,"link",0)"#],
    );
    assert_compiles_to(
        r#"<link rel="icon" href="./favicon.ico">"#,
        &[r#"i0.ɵɵelement(0,"link",0)"#],
    );
}

/// `isNgContent` reads the namespace-stripped local name lowercased, so both
/// a namespaced `<svg:ng-content>` and a capitalised `<NG-CONTENT>` project
/// content, and its `select` attribute is matched case-insensitively.
#[test]
fn namespaced_and_capitalised_ng_content() {
    assert_compiles_to(
        "<svg:ng-content SELECT=\"[a]\"></svg:ng-content><NG-CONTENT />",
        &[
            "ngContentSelectors:_c1,",
            "i0.ɵɵprojectionDef(_c0);i0.ɵɵprojection(0);i0.ɵɵprojection(1,1);",
        ],
    );
}

/// `isNgTemplate` reads the namespace-stripped local name as written, so a
/// namespaced `<svg:ng-template>` is a template while a capitalised
/// `<NG-TEMPLATE>` is an ordinary element.
#[test]
fn namespaced_ng_template_vs_capitalised() {
    assert_compiles_to("<svg:ng-template><p>x</p></svg:ng-template>", &["decls:"]);
    assert_compiles_to(
        "<NG-TEMPLATE>a</NG-TEMPLATE>",
        &[r#"i0.ɵɵelementStart(0,"NG-TEMPLATE");i0.ɵɵtext(1,"a")"#],
    );
}
