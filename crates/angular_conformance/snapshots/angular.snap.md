# Angular Conformance Test Results

## Summary

| Subsystem | Passed | Failed | Errors | Skipped | Total | Pass Rate |
|-----------|--------|--------|--------|---------|-------|-----------|
| ast_serializer | 6 | 0 | 0 | 0 | 6 | 100.0% |
| ast_spans | 55 | 0 | 0 | 0 | 55 | 100.0% |
| expression_lexer | 137 | 0 | 0 | 0 | 137 | 100.0% |
| expression_parser | 270 | 0 | 0 | 0 | 270 | 100.0% |
| expression_serializer | 38 | 0 | 0 | 0 | 38 | 100.0% |
| html_lexer | 286 | 3 | 0 | 0 | 289 | 99.0% |
| html_parser | 85 | 2 | 0 | 0 | 87 | 97.7% |
| html_whitespace | 21 | 0 | 0 | 0 | 21 | 100.0% |
| r3_transform | 168 | 9 | 0 | 0 | 177 | 94.9% |
| shadow_css | 160 | 9 | 0 | 0 | 169 | 94.7% |
| style_parser | 15 | 0 | 0 | 0 | 15 | 100.0% |
| **Total** | **1241** | **23** | **0** | **0** | **1264** | **98.2%** |

## Failed Tests

### html_lexer

#### should parse @default never;: HtmlLexerTest { input: "@default never;", test_type: HumanizeParts, expected: [Array [String("TokenType.BLOCK_OPEN_START"), String("default never")], Array [String("TokenType.BLOCK_OPEN_END")], Array [String("TokenType.BLOCK_CLOSE")], Array [String("TokenType.EOF")]], options: None }
Path: `HtmlLexer/blocks/should parse @default never;`

**Expected:**
```
4 tokens
```

**Actual:**
```
3 tokens: [Array [String("TokenType.INCOMPLETE_BLOCK_OPEN"), String("default never")], Array [String("TokenType.TEXT"), String(";")], Array [String("TokenType.EOF")]]
```

#### should parse @default never(expr);: HtmlLexerTest { input: "@default never(expr);", test_type: HumanizeParts, expected: [Array [String("TokenType.BLOCK_OPEN_START"), String("default never")], Array [String("TokenType.BLOCK_PARAMETER"), String("expr")], Array [String("TokenType.BLOCK_OPEN_END")], Array [String("TokenType.BLOCK_CLOSE")], Array [String("TokenType.EOF")]], options: None }
Path: `HtmlLexer/blocks/should parse @default never(expr);`

**Expected:**
```
5 tokens
```

**Actual:**
```
4 tokens: [Array [String("TokenType.INCOMPLETE_BLOCK_OPEN"), String("default never")], Array [String("TokenType.BLOCK_PARAMETER"), String("expr")], Array [String("TokenType.TEXT"), String(";")], Array [String("TokenType.EOF")]]
```

#### should parse @default never ;: HtmlLexerTest { input: "@default never ;", test_type: HumanizeParts, expected: [Array [String("TokenType.BLOCK_OPEN_START"), String("default never")], Array [String("TokenType.BLOCK_OPEN_END")], Array [String("TokenType.BLOCK_CLOSE")], Array [String("TokenType.EOF")]], options: None }
Path: `HtmlLexer/blocks/should parse @default never ;`

**Expected:**
```
4 tokens
```

**Actual:**
```
3 tokens: [Array [String("TokenType.INCOMPLETE_BLOCK_OPEN"), String("default never")], Array [String("TokenType.TEXT"), String(";")], Array [String("TokenType.EOF")]]
```

### html_parser

#### should parse exhaustive default checks in a switch block: HumanizeDom { input: "@switch (expr) {@case ('foo') {} @default never;}", expected: [Array [String("html.Block"), String("switch"), Number(0.0)], Array [String("html.BlockParameter"), String("expr")], Array [String("html.Block"), String("case"), Number(1.0)], Array [String("html.BlockParameter"), String("'foo'")], Array [String("html.Text"), String(" "), Number(1.0), Array [String(" ")]], Array [String("html.Block"), String("default never"), Number(1.0)]] }
Path: `HtmlParser/parse/blocks/should parse exhaustive default checks in a switch block`

**Expected:**
```
[Array [String("html.Block"), String("switch"), Number(0.0)], Array [String("html.BlockParameter"), String("expr")], Array [String("html.Block"), String("case"), Number(1.0)], Array [String("html.BlockParameter"), String("'foo'")], Array [String("html.Text"), String(" "), Number(1.0), Array [String(" ")]], Array [String("html.Block"), String("default never"), Number(1.0)]]
```

**Actual:**
```
[Array [String("html.Block"), String("switch"), Number(0)], Array [String("html.BlockParameter"), String("expr")], Array [String("html.Block"), String("case"), Number(1)], Array [String("html.BlockParameter"), String("'foo'")], Array [String("html.Text"), String(" "), Number(1), Array [String(" ")]], Array [String("html.Text"), String(";"), Number(1), Array [String(";")]]]
```

#### should store the source location of a @let declaration: HumanizeDomSourceSpans { input: "@let foo = 123 + 456;", expected: [Array [String("html.LetDeclaration"), String("foo"), String("123 + 456"), String("@let foo = 123 + 456;"), String("foo"), String("123 + 456")]], options: None }
Path: `HtmlParser/parse/let declaration/should store the source location of a @let declaration`

**Expected:**
```
[Array [String("html.LetDeclaration"), String("foo"), String("123 + 456"), String("@let foo = 123 + 456;"), String("foo"), String("123 + 456")]]
```

**Actual:**
```
[Array [String("html.LetDeclaration"), String("foo"), String("123 + 456"), String("@let foo = 123 + 456"), String("foo"), String("123 + 456")]]
```

### r3_transform

#### is correct for bound properties via data-: ExpectFromHtml { input: "<div data-prop=\"{{v}}\"></div>", expected: [Array [String("Element"), String("<div data-prop=\"{{v}}\"></div>"), String("<div data-prop=\"{{v}}\">"), String("</div>")], Array [String("BoundAttribute"), String("data-prop=\"{{v}}\""), String("data-prop"), String("{{v}}")]], ignore_error: false }
Path: `R3 AST source spans/bound attributes/is correct for bound properties via data-`

**Expected:**
```
[Element, <div data-prop="{{v}}"></div>, <div data-prop="{{v}}">, </div>]
[BoundAttribute, data-prop="{{v}}", data-prop, {{v}}]
```

**Actual:**
```
[Element, <div data-prop="{{v}}"></div>, <div data-prop="{{v}}">, </div>]
[BoundAttribute, data-prop="{{v}}", prop, {{v}}]
```

**Diff:**
```diff
 [Element, <div data-prop="{{v}}"></div>, <div data-prop="{{v}}">, </div>]
-[BoundAttribute, data-prop="{{v}}", data-prop, {{v}}]
+[BoundAttribute, data-prop="{{v}}", prop, {{v}}]

```

#### is correct for data-ref-... attribute: ExpectFromHtml { input: "<ng-template data-ref-a></ng-template>", expected: [Array [String("Template"), String("<ng-template data-ref-a></ng-template>"), String("<ng-template data-ref-a>"), String("</ng-template>")], Array [String("TextAttribute"), String("data-ref-a"), String("data-ref-a"), String("<empty>")]], ignore_error: false }
Path: `R3 AST source spans/templates/is correct for data-ref-... attribute`

**Expected:**
```
[Template, <ng-template data-ref-a></ng-template>, <ng-template data-ref-a>, </ng-template>]
[TextAttribute, data-ref-a, data-ref-a, <empty>]
```

**Actual:**
```
[Template, <ng-template data-ref-a></ng-template>, <ng-template data-ref-a>, </ng-template>]
[Reference, data-ref-a, a, <empty>]
```

**Diff:**
```diff
 [Template, <ng-template data-ref-a></ng-template>, <ng-template data-ref-a>, </ng-template>]
-[TextAttribute, data-ref-a, data-ref-a, <empty>]
+[Reference, data-ref-a, a, <empty>]

```

#### is correct for data-let-... attribute: ExpectFromHtml { input: "<ng-template data-let-a=\"b\"></ng-template>", expected: [Array [String("Template"), String("<ng-template data-let-a=\"b\"></ng-template>"), String("<ng-template data-let-a=\"b\">"), String("</ng-template>")], Array [String("TextAttribute"), String("data-let-a=\"b\""), String("data-let-a"), String("b")]], ignore_error: false }
Path: `R3 AST source spans/templates/is correct for data-let-... attribute`

**Expected:**
```
[Template, <ng-template data-let-a="b"></ng-template>, <ng-template data-let-a="b">, </ng-template>]
[TextAttribute, data-let-a="b", data-let-a, b]
```

**Actual:**
```
[Template, <ng-template data-let-a="b"></ng-template>, <ng-template data-let-a="b">, </ng-template>]
[Variable, data-let-a="b", a, b]
```

**Diff:**
```diff
 [Template, <ng-template data-let-a="b"></ng-template>, <ng-template data-let-a="b">, </ng-template>]
-[TextAttribute, data-let-a="b", data-let-a, b]
+[Variable, data-let-a="b", a, b]

```

#### is correct for text attribute via data-on-: ExpectFromHtml { input: "<div data-on-event=\"v\"></div>", expected: [Array [String("Element"), String("<div data-on-event=\"v\"></div>"), String("<div data-on-event=\"v\">"), String("</div>")], Array [String("TextAttribute"), String("data-on-event=\"v\""), String("data-on-event"), String("v")]], ignore_error: false }
Path: `R3 AST source spans/events/is correct for text attribute via data-on-`

**Expected:**
```
[Element, <div data-on-event="v"></div>, <div data-on-event="v">, </div>]
[TextAttribute, data-on-event="v", data-on-event, v]
```

**Actual:**
```
[Element, <div data-on-event="v"></div>, <div data-on-event="v">, </div>]
[BoundEvent, data-on-event="v", event, v]
```

**Diff:**
```diff
 [Element, <div data-on-event="v"></div>, <div data-on-event="v">, </div>]
-[TextAttribute, data-on-event="v", data-on-event, v]
+[BoundEvent, data-on-event="v", event, v]

```

#### is correct for TextAttribute and properties via data-bindon-: ExpectFromHtml { input: "<div data-bindon-prop=\"v\"></div>", expected: [Array [String("Element"), String("<div data-bindon-prop=\"v\"></div>"), String("<div data-bindon-prop=\"v\">"), String("</div>")], Array [String("TextAttribute"), String("data-bindon-prop=\"v\""), String("data-bindon-prop"), String("v")]], ignore_error: false }
Path: `R3 AST source spans/events/is correct for TextAttribute and properties via data-bindon-`

**Expected:**
```
[Element, <div data-bindon-prop="v"></div>, <div data-bindon-prop="v">, </div>]
[TextAttribute, data-bindon-prop="v", data-bindon-prop, v]
```

**Actual:**
```
[Element, <div data-bindon-prop="v"></div>, <div data-bindon-prop="v">, </div>]
[BoundAttribute, data-bindon-prop="v", prop, v]
[BoundEvent, data-bindon-prop="v", prop, v]
```

**Diff:**
```diff
 [Element, <div data-bindon-prop="v"></div>, <div data-bindon-prop="v">, </div>]
-[TextAttribute, data-bindon-prop="v", data-bindon-prop, v]
+[BoundAttribute, data-bindon-prop="v", prop, v]
+[BoundEvent, data-bindon-prop="v", prop, v]

```

#### is correct for switch blocks with exhaustive checking: ExpectFromHtml { input: "@switch (cond.kind) {@case (x()) {X case}@default never;}", expected: [Array [String("SwitchBlock"), String("@switch (cond.kind) {@case (x()) {X case}@default never;}"), String("@switch (cond.kind) {"), String("}")], Array [String("SwitchBlockCaseGroup"), String("@case (x()) {X case}"), String("@case (x()) {")], Array [String("SwitchBlockCase"), String("@case (x()) {X case}"), String("@case (x()) {")], Array [String("Text"), String("X case")], Array [String("SwitchExhaustiveCheck"), String("@default never;"), String("@default never;")]], ignore_error: false }
Path: `R3 AST source spans/switch blocks/is correct for switch blocks with exhaustive checking`

**Expected:**
```
[SwitchBlock, @switch (cond.kind) {@case (x()) {X case}@default never;}, @switch (cond.kind) {, }]
[SwitchBlockCaseGroup, @case (x()) {X case}, @case (x()) {]
[SwitchBlockCase, @case (x()) {X case}, @case (x()) {]
[Text, X case]
[SwitchExhaustiveCheck, @default never;, @default never;]
```

**Actual:**
```
[SwitchBlock, @switch (cond.kind) {@case (x()) {X case}@default never;}, @switch (cond.kind) {, }]
[SwitchBlockCaseGroup, @case (x()) {X case}, @case (x()) {]
[SwitchBlockCase, @case (x()) {X case}, @case (x()) {]
[Text, X case]
```

**Diff:**
```diff
 [SwitchBlock, @switch (cond.kind) {@case (x()) {X case}@default never;}, @switch (cond.kind) {, }]
 [SwitchBlockCaseGroup, @case (x()) {X case}, @case (x()) {]
 [SwitchBlockCase, @case (x()) {X case}, @case (x()) {]
-[Text, X case]
-[SwitchExhaustiveCheck, @default never;, @default never;]
+[Text, X case]

```

#### is correct for a let declaration: ExpectFromHtml { input: "@let foo = 123;", expected: [Array [String("LetDeclaration"), String("@let foo = 123;"), String("foo"), String("123")]], ignore_error: false }
Path: `R3 AST source spans/@let declaration/is correct for a let declaration`

**Expected:**
```
[LetDeclaration, @let foo = 123;, foo, 123]
```

**Actual:**
```
[LetDeclaration, @let foo = 123, foo, 123]
```

**Diff:**
```diff
-[LetDeclaration, @let foo = 123;, foo, 123]
+[LetDeclaration, @let foo = 123, foo, 123]

```

#### should not ignore namespaced SVG <style> elements: ExpectFromHtml { input: "<svg><style>.a { fill: none; }</style></svg>", expected: [Array [String("Element"), String(":svg:svg")], Array [String("Element"), String(":svg:style")], Array [String("Text"), String(".a { fill: none; }")]], ignore_error: false }
Path: `R3 template transform/<script> and <style> elements/should not ignore namespaced SVG <style> elements`

**Expected:**
```
[Element, :svg:svg]
[Element, :svg:style]
[Text, .a { fill: none; }]
```

**Actual:**
```
[Element, :svg:svg]
```

**Diff:**
```diff
-[Element, :svg:svg]
-[Element, :svg:style]
-[Text, .a { fill: none; }]
+[Element, :svg:svg]

```

#### should parse a switch block with a default never case: ExpectFromHtml { input: "\n          @switch (cond.kind) {\n            @default never;\n          }\n        ", expected: [Array [String("SwitchBlock"), String("cond.kind")], Array [String("SwitchExhaustiveCheck")]], ignore_error: false }
Path: `R3 template transform/switch blocks/should parse a switch block with a default never case`

**Expected:**
```
[SwitchBlock, cond.kind]
[SwitchExhaustiveCheck]
```

**Actual:**
```
[SwitchBlock, cond.kind]
```

**Diff:**
```diff
-[SwitchBlock, cond.kind]
-[SwitchExhaustiveCheck]
+[SwitchBlock, cond.kind]

```

### shadow_css

#### should ignore :host with a selector list containing top-level commas: ShimCss { input: ":host(.a, .b) {}", content_attr: "contenta", host_attr: Some("a-host"), expected: "[contenta]:host(.a, .b) {}", normalized: true }
Path: `ShadowCss, :host and :host-context/:host/should ignore :host with a selector list containing top-level commas`

**Expected:**
```
[contenta]:host(.a, .b) {}
```

**Actual:**
```
.a[a-host], .b[a-host] {}
```

**Diff:**
```diff
Normalized comparison: expected='[contenta]:host(.a, .b){}', actual='.a[a-host], .b[a-host]{}'
```

#### should ignore :host with a selector list containing top-level commas: ShimCss { input: ".outer :host(.a, .b) .inner {}", content_attr: "contenta", host_attr: Some("a-host"), expected: ".outer[contenta] [contenta]:host(.a, .b) .inner[contenta] {}", normalized: true }
Path: `ShadowCss, :host and :host-context/:host/should ignore :host with a selector list containing top-level commas`

**Expected:**
```
.outer[contenta] [contenta]:host(.a, .b) .inner[contenta] {}
```

**Actual:**
```
.outer .a[a-host] .inner[contenta], .b[a-host] .inner[contenta] {}
```

**Diff:**
```diff
Normalized comparison: expected='.outer[contenta] [contenta]:host(.a, .b) .inner[contenta]{}', actual='.outer .a[a-host] .inner[contenta], .b[a-host] .inner[contenta]{}'
```

#### should handle :host-context with no ancestor selectors: ShimCss { input: ":host-context .inner {}", content_attr: "contenta", host_attr: Some("a-host"), expected: "[contenta]:host-context .inner[contenta] {}", normalized: true }
Path: `ShadowCss, :host and :host-context/:host-context/should handle :host-context with no ancestor selectors`

**Expected:**
```
[contenta]:host-context .inner[contenta] {}
```

**Actual:**
```
[a-host] .inner[contenta] {}
```

**Diff:**
```diff
Normalized comparison: expected='[contenta]:host-context .inner[contenta]{}', actual='[a-host] .inner[contenta]{}'
```

#### should handle :host-context with no ancestor selectors: ShimCss { input: ":host-context() .inner {}", content_attr: "contenta", host_attr: Some("a-host"), expected: "[contenta]:host-context() .inner[contenta] {}", normalized: true }
Path: `ShadowCss, :host and :host-context/:host-context/should handle :host-context with no ancestor selectors`

**Expected:**
```
[contenta]:host-context() .inner[contenta] {}
```

**Actual:**
```
[a-host] .inner[contenta] {}
```

**Diff:**
```diff
Normalized comparison: expected='[contenta]:host-context() .inner[contenta]{}', actual='[a-host] .inner[contenta]{}'
```

#### should handle :host-context with no ancestor selectors: ShimCss { input: ":host-context :host-context(.a) {}", content_attr: "contenta", host_attr: Some("host-a"), expected: ":host-context .a[host-a], .a [host-a] {}", normalized: true }
Path: `ShadowCss, :host and :host-context/:host-context/should handle :host-context with no ancestor selectors`

**Expected:**
```
:host-context .a[host-a], .a [host-a] {}
```

**Actual:**
```
.a[host-a], .a [host-a] {}
```

**Diff:**
```diff
Normalized comparison: expected=':host-context .a[host-a], .a [host-a]{}', actual='.a[host-a], .a [host-a]{}'
```

#### should remove inline comments without adding extra lines: ShimCss { input: "/* b {} */ b {}", content_attr: "contenta", host_attr: None, expected: " b[contenta] {}", normalized: false }
Path: `ShadowCss/comments/should remove inline comments without adding extra lines`

**Expected:**
```
 b[contenta] {}
```

**Actual:**
```

 b[contenta] {}
```

**Diff:**
```diff
Normalized comparison: expected=' b[contenta] {}', actual='
 b[contenta] {}'
```

#### should preserve internal newlines from multiline comments: ShimCss { input: "/* b {}\n */ b {}", content_attr: "contenta", host_attr: None, expected: "\n b[contenta] {}", normalized: false }
Path: `ShadowCss/comments/should preserve internal newlines from multiline comments`

**Expected:**
```

 b[contenta] {}
```

**Actual:**
```


 b[contenta] {}
```

**Diff:**
```diff
Normalized comparison: expected='
 b[contenta] {}', actual='

 b[contenta] {}'
```

#### should remove multiple inline comments without adding extra lines: ShimCss { input: "/* b {} */ b {} /* a {} */ a {}", content_attr: "contenta", host_attr: None, expected: " b[contenta] {}  a[contenta] {}", normalized: false }
Path: `ShadowCss/comments/should remove multiple inline comments without adding extra lines`

**Expected:**
```
 b[contenta] {}  a[contenta] {}
```

**Actual:**
```

 b[contenta] {} 
 a[contenta] {}
```

**Diff:**
```diff
Normalized comparison: expected=' b[contenta] {}  a[contenta] {}', actual='
 b[contenta] {} 
 a[contenta] {}'
```

#### should handle adjacent comments: ShimCss { input: "/* comment 1 */ /* comment 2 */ b {}", content_attr: "contenta", host_attr: None, expected: "  b[contenta] {}", normalized: false }
Path: `ShadowCss/comments/should handle adjacent comments`

**Expected:**
```
  b[contenta] {}
```

**Actual:**
```

 
 b[contenta] {}
```

**Diff:**
```diff
Normalized comparison: expected='  b[contenta] {}', actual='
 
 b[contenta] {}'
```

