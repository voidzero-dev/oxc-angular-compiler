# Angular Conformance Test Results

## Summary

| Subsystem | Passed | Failed | Errors | Skipped | Total | Pass Rate |
|-----------|--------|--------|--------|---------|-------|-----------|
| ast_serializer | 6 | 0 | 0 | 0 | 6 | 100.0% |
| ast_spans | 55 | 0 | 0 | 0 | 55 | 100.0% |
| expression_lexer | 154 | 0 | 0 | 0 | 154 | 100.0% |
| expression_parser | 390 | 10 | 3 | 0 | 403 | 96.8% |
| expression_serializer | 44 | 0 | 0 | 0 | 44 | 100.0% |
| html_lexer | 291 | 6 | 0 | 0 | 297 | 98.0% |
| html_parser | 89 | 1 | 0 | 0 | 90 | 98.9% |
| html_whitespace | 21 | 0 | 0 | 0 | 21 | 100.0% |
| r3_transform | 180 | 4 | 0 | 0 | 184 | 97.8% |
| shadow_css | 185 | 2 | 0 | 0 | 187 | 98.9% |
| style_parser | 15 | 0 | 0 | 0 | 15 | 100.0% |
| **Total** | **1430** | **23** | **3** | **0** | **1456** | **98.2%** |

## Failed Tests

### expression_parser

#### should report error if interpolation is empty: CheckBinding { input: "`hello ${}`", expected: None }
Path: `parser/parseAction/template literals/should report error if interpolation is empty`

**Error:**
```
Unexpected parse errors: ["Template literal interpolation cannot be empty"]
```

#### should report an error if an interpolation has unexpected tokens after the expression: ExpectBindingError { input: "`hello ${a b} world`", error_contains: "Missing expected }" }
Path: `parser/parseAction/template literals/should report an error if an interpolation has unexpected tokens after the expression`

**Expected:**
```
Error containing: Missing expected }
```

**Actual:**
```
Errors: ["Unexpected token 'b' at column 12 in [`hello ${a b} world`]"]
```

#### should report an error if an interpolation has unexpected tokens after the expression: ExpectBindingError { input: "`hello ${a & b} world`", error_contains: "Missing expected }" }
Path: `parser/parseAction/template literals/should report an error if an interpolation has unexpected tokens after the expression`

**Expected:**
```
Error containing: Missing expected }
```

**Actual:**
```
Errors: ["Unexpected token '&' at column 12 in [`hello ${a & b} world`]"]
```

#### should report an error if an interpolation has unexpected tokens after the expression: ExpectBindingError { input: "`hello ${a, b} world`", error_contains: "Missing expected }" }
Path: `parser/parseAction/template literals/should report an error if an interpolation has unexpected tokens after the expression`

**Expected:**
```
Error containing: Missing expected }
```

**Actual:**
```
Errors: ["Unexpected token ',' at column 11 in [`hello ${a, b} world`]"]
```

#### should report an error if an interpolation has unexpected tokens after the expression: ExpectBindingError { input: "`hello ${`nested ${a b}`} world`", error_contains: "Missing expected }" }
Path: `parser/parseAction/template literals/should report an error if an interpolation has unexpected tokens after the expression`

**Expected:**
```
Error containing: Missing expected }
```

**Actual:**
```
Errors: ["Unexpected token 'b' at column 22 in [`hello ${`nested ${a b}`} world`]"]
```

#### should report an error if an interpolation has unexpected tokens after the expression: ExpectBindingError { input: "tag`hello ${a b} world`", error_contains: "Missing expected }" }
Path: `parser/parseAction/template literals/should report an error if an interpolation has unexpected tokens after the expression`

**Expected:**
```
Error containing: Missing expected }
```

**Actual:**
```
Errors: ["Unexpected token 'b' at column 15 in [tag`hello ${a b} world`]"]
```

#### should report an error if an interpolation has unexpected tokens after the expression: ExpectBindingError { input: "`hello ${a | pipe b} world`", error_contains: "Missing expected }" }
Path: `parser/parseAction/template literals/should report an error if an interpolation has unexpected tokens after the expression`

**Expected:**
```
Error containing: Missing expected }
```

**Actual:**
```
Errors: ["Unexpected token 'b' at column 19 in [`hello ${a | pipe b} world`]"]
```

#### should report an error if an interpolation has unexpected tokens after the expression: ExpectActionError { input: "`hello ${a; b} world`", error_contains: "Missing expected }" }
Path: `parser/parseAction/template literals/should report an error if an interpolation has unexpected tokens after the expression`

**Expected:**
```
Error containing: Missing expected }
```

**Actual:**
```
Errors: ["Unexpected token '}' at column 14 in [`hello ${a; b} world`]"]
```

#### should report an error if an interpolation is not closed: ExpectBindingError { input: "`hello ${a", error_contains: "Missing expected } at the end of the expression" }
Path: `parser/parseAction/template literals/should report an error if an interpolation is not closed`

**Expected:**
```
Error containing: Missing expected } at the end of the expression
```

**Actual:**
```
No errors
```

#### should report an error if an interpolation is not closed: ExpectBindingError { input: "`hello ${a ", error_contains: "Missing expected } at the end of the expression" }
Path: `parser/parseAction/template literals/should report an error if an interpolation is not closed`

**Expected:**
```
Error containing: Missing expected } at the end of the expression
```

**Actual:**
```
No errors
```

#### should report an error if a template literal with an interpolation is not closed: ExpectBindingError { input: "`hello ${a}", error_contains: "Unterminated template literal" }
Path: `parser/parseAction/template literals/should report an error if a template literal with an interpolation is not closed`

**Expected:**
```
Error containing: Unterminated template literal
```

**Actual:**
```
No errors
```

#### should parse a regular expression literal that ends with an escaped slash: CheckBinding { input: "/^https?:\\/\\//", expected: None }
Path: `parser/parseAction/regular expression literals/should parse a regular expression literal that ends with an escaped slash`

**Error:**
```
Unexpected parse errors: ["Lexer Error: Unterminated regular expression at column 12 in expression [/^https?:\\/\\]", "Unexpected token 'Lexer Error: Unterminated regular expression at column 12 in expression [/^https?:\\/\\]' at column 13 in [/^https?:\\/\\//]"]
```

#### should parse a regular expression literal that ends with an escaped slash: CheckBinding { input: "url.replace(/^\\//, \"\") //comment", expected: Some("url.replace(/^\\//, \"\")") }
Path: `parser/parseAction/regular expression literals/should parse a regular expression literal that ends with an escaped slash`

**Error:**
```
Unexpected parse errors: ["Lexer Error: Unterminated regular expression at column 15 in expression [url.replace(/^\\]", "Missing expected ) at column 16"]
```

### html_lexer

#### should parse a processing instruction: HtmlLexerTest { input: "<?xml version=\"1.0\" encoding=\"UTF-8\"?>", test_type: HumanizeParts, expected: [Array [String("TokenType.PROCESSING_INSTRUCTION"), String("xml version=\"1.0\" encoding=\"UTF-8\"")], Array [String("TokenType.EOF")]], options: None }
Path: `HtmlLexer/processing instructions/should parse a processing instruction`

**Expected:**
```
Token 0: Array [String("TokenType.PROCESSING_INSTRUCTION"), String("xml version=\"1.0\" encoding=\"UTF-8\"")]
```

**Actual:**
```
Token 0: Array [String("TokenType.TEXT"), String("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")]
```

#### should parse a processing instruction ending with > instead of ?>: HtmlLexerTest { input: "<?xml version=\"1.0\" encoding=\"UTF-8\">", test_type: HumanizeParts, expected: [Array [String("TokenType.PROCESSING_INSTRUCTION"), String("xml version=\"1.0\" encoding=\"UTF-8\"")], Array [String("TokenType.EOF")]], options: None }
Path: `HtmlLexer/processing instructions/should parse a processing instruction ending with > instead of ?>`

**Expected:**
```
Token 0: Array [String("TokenType.PROCESSING_INSTRUCTION"), String("xml version=\"1.0\" encoding=\"UTF-8\"")]
```

**Actual:**
```
Token 0: Array [String("TokenType.TEXT"), String("<?xml version=\"1.0\" encoding=\"UTF-8\">")]
```

#### should parse a processing instruction surrounded by other content: HtmlLexerTest { input: "<!DOCTYPE html>\n<?xml version=\"1.0\" encoding=\"UTF-8\"?> hello", test_type: HumanizeParts, expected: [Array [String("TokenType.DOC_TYPE"), String("DOCTYPE html")], Array [String("TokenType.TEXT"), String("\n")], Array [String("TokenType.PROCESSING_INSTRUCTION"), String("xml version=\"1.0\" encoding=\"UTF-8\"")], Array [String("TokenType.TEXT"), String(" hello")], Array [String("TokenType.EOF")]], options: None }
Path: `HtmlLexer/processing instructions/should parse a processing instruction surrounded by other content`

**Expected:**
```
5 tokens
```

**Actual:**
```
3 tokens: [Array [String("TokenType.DOC_TYPE"), String("DOCTYPE html")], Array [String("TokenType.TEXT"), String("\n<?xml version=\"1.0\" encoding=\"UTF-8\"?> hello")], Array [String("TokenType.EOF")]]
```

#### should parse a processing instruction that contains question marks inside quoted content: HtmlLexerTest { input: "<?xml version=\"?\" encoding=\"UTF?>-8\"?>", test_type: HumanizeParts, expected: [Array [String("TokenType.PROCESSING_INSTRUCTION"), String("xml version=\"?\" encoding=\"UTF?>-8\"")], Array [String("TokenType.EOF")]], options: None }
Path: `HtmlLexer/processing instructions/should parse a processing instruction that contains question marks inside quoted content`

**Expected:**
```
Token 0: Array [String("TokenType.PROCESSING_INSTRUCTION"), String("xml version=\"?\" encoding=\"UTF?>-8\"")]
```

**Actual:**
```
Token 0: Array [String("TokenType.TEXT"), String("<?xml version=\"?\" encoding=\"UTF?>-8\"?>")]
```

#### should store the location of a processing instruction: HtmlLexerTest { input: "<?xml version=\"1.0\" encoding=\"UTF-8\"?>", test_type: HumanizeSourceSpans, expected: [Array [String("TokenType.PROCESSING_INSTRUCTION"), String("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")], Array [String("TokenType.EOF"), String("")]], options: None }
Path: `HtmlLexer/processing instructions/should store the location of a processing instruction`

**Expected:**
```
Token 0: Array [String("TokenType.PROCESSING_INSTRUCTION"), String("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")]
```

**Actual:**
```
Token 0: Array [String("TokenType.TEXT"), String("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")]
```

#### should report missing end of a processing instruction: HtmlLexerTest { input: "<?", test_type: HumanizeErrors, expected: [Array [String("Unexpected character \"EOF\""), String("0:2")]], options: None }
Path: `HtmlLexer/processing instructions/should report missing end of a processing instruction`

**Expected:**
```
1 errors
```

**Actual:**
```
0 errors
```

### html_parser

#### should ignore processing instruction: HumanizeDom { input: "<?xml version=\"1.0\" encoding=\"UTF-8\"?>hello", expected: [Array [String("html.Text"), String("hello"), Number(0.0), Array [String("hello")]]] }
Path: `HtmlParser/parse/ignored syntax/should ignore processing instruction`

**Expected:**
```
[Array [String("html.Text"), String("hello"), Number(0.0), Array [String("hello")]]]
```

**Actual:**
```
[Array [String("html.Text"), String("<?xml version=\"1.0\" encoding=\"UTF-8\"?>hello"), Number(0), Array [String("<?xml version=\"1.0\" encoding=\"UTF-8\"?>hello")]]]
```

### r3_transform

#### should ignore <script> elements inherited into custom namespaces: ExpectFromHtml { input: "<xhtml:div xmlns:xhtml=\"http://www.w3.org/1999/xhtml\"><script [textContent]=\"evil\"></script><span>safe</span></xhtml:div>", expected: [Array [String("Element"), String(":xhtml:div")], Array [String("TextAttribute"), String(":xmlns:xhtml"), String("http://www.w3.org/1999/xhtml")], Array [String("Element"), String(":xhtml:span")], Array [String("Text"), String("safe")]], ignore_error: false }
Path: `R3 template transform/<script> and <style> elements/should ignore <script> elements inherited into custom namespaces`

**Expected:**
```
[Element, :xhtml:div]
[TextAttribute, :xmlns:xhtml, http://www.w3.org/1999/xhtml]
[Element, :xhtml:span]
[Text, safe]
```

**Actual:**
```
[Element, :xhtml:div]
[TextAttribute, :xmlns:xhtml, http://www.w3.org/1999/xhtml]
[Element, :xhtml:script]
[BoundAttribute, 0, textContent, evil]
[Element, :xhtml:span]
[Text, safe]
```

**Diff:**
```diff
 [Element, :xhtml:div]
 [TextAttribute, :xmlns:xhtml, http://www.w3.org/1999/xhtml]
+[Element, :xhtml:script]
+[BoundAttribute, 0, textContent, evil]
 [Element, :xhtml:span]
 [Text, safe]

```

#### should ignore custom-namespaced <script> elements inside of elements with ngNonBindable: ExpectFromHtml { input: "<foo:div ngNonBindable><script>evil</script><span>safe</span></foo:div>", expected: [Array [String("Element"), String(":foo:div")], Array [String("TextAttribute"), String("ngNonBindable"), String("")], Array [String("Element"), String(":foo:span")], Array [String("Text"), String("safe")]], ignore_error: false }
Path: `R3 template transform/ngNonBindable/should ignore custom-namespaced <script> elements inside of elements with ngNonBindable`

**Expected:**
```
[Element, :foo:div]
[TextAttribute, ngNonBindable, ]
[Element, :foo:span]
[Text, safe]
```

**Actual:**
```
[Element, :foo:div]
[TextAttribute, ngNonBindable, ]
[Element, :foo:script]
[Text, evil]
[Element, :foo:span]
[Text, safe]
```

**Diff:**
```diff
 [Element, :foo:div]
 [TextAttribute, ngNonBindable, ]
+[Element, :foo:script]
+[Text, evil]
 [Element, :foo:span]
 [Text, safe]

```

#### should parse a valid @content block: ExpectFromHtml { input: "\n        @content(icon) {\n          <span>Icon content</span>\n        }\n      ", expected: [Array [String("ContentBlock"), String("icon")], Array [String("Element"), String("span")], Array [String("Text"), String("Icon content")]], ignore_error: false }
Path: `R3 template transform/@content blocks/should parse a valid @content block`

**Expected:**
```
[ContentBlock, icon]
[Element, span]
[Text, Icon content]
```

**Actual:**
```

```

**Diff:**
```diff
-[ContentBlock, icon]
-[Element, span]
-[Text, Icon content]

```

#### should parse a valid @content block with variables: ExpectFromHtml { input: "\n        @content(icon; let a, b) {\n          <span>Icon content</span>\n        }\n      ", expected: [Array [String("ContentBlock"), String("icon")], Array [String("Variable"), String("a"), String("")], Array [String("Variable"), String("b"), String("")], Array [String("Element"), String("span")], Array [String("Text"), String("Icon content")]], ignore_error: false }
Path: `R3 template transform/@content blocks/should parse a valid @content block with variables`

**Expected:**
```
[ContentBlock, icon]
[Variable, a, ]
[Variable, b, ]
[Element, span]
[Text, Icon content]
```

**Actual:**
```

```

**Diff:**
```diff
-[ContentBlock, icon]
-[Variable, a, ]
-[Variable, b, ]
-[Element, span]
-[Text, Icon content]

```

### shadow_css

#### should disable encapsulation of nested selectors if the parent selector ends with ::ng-deep: ShimCss { input: ".foo { .bar ::ng-deep { .baz { color: red; } } }", content_attr: "contenta", host_attr: None, expected: ".foo[contenta] { .bar[contenta] { .baz { color: red; } } }", normalized: true }
Path: `ShadowCss nesting/should disable encapsulation of nested selectors if the parent selector ends with ::ng-deep`

**Expected:**
```
.foo[contenta] { .bar[contenta] { .baz { color: red; } } }
```

**Actual:**
```
.foo[contenta] { .bar ::ng-deep { .baz { color: red; } } }
```

**Diff:**
```diff
Normalized comparison: expected='.foo[contenta]{ .bar[contenta]{ .baz{ color:red;}}}', actual='.foo[contenta]{ .bar ::ng-deep{ .baz{ color:red;}}}'
```

#### should disable encapsulation of nested selectors if the parent selector starts with ::ng-deep: ShimCss { input: ".foo { ::ng-deep .bar { .baz { color: red; } } }", content_attr: "contenta", host_attr: None, expected: ".foo[contenta] { .bar { .baz { color: red; } } }", normalized: true }
Path: `ShadowCss nesting/should disable encapsulation of nested selectors if the parent selector starts with ::ng-deep`

**Expected:**
```
.foo[contenta] { .bar { .baz { color: red; } } }
```

**Actual:**
```
.foo[contenta] { ::ng-deep .bar { .baz { color: red; } } }
```

**Diff:**
```diff
Normalized comparison: expected='.foo[contenta]{ .bar{ .baz{ color:red;}}}', actual='.foo[contenta]{ ::ng-deep .bar{ .baz{ color:red;}}}'
```

