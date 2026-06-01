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
| html_parser | 86 | 1 | 0 | 0 | 87 | 98.9% |
| html_whitespace | 21 | 0 | 0 | 0 | 21 | 100.0% |
| r3_transform | 175 | 2 | 0 | 0 | 177 | 98.9% |
| shadow_css | 169 | 0 | 0 | 0 | 169 | 100.0% |
| style_parser | 15 | 0 | 0 | 0 | 15 | 100.0% |
| **Total** | **1258** | **6** | **0** | **0** | **1264** | **99.5%** |

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

### r3_transform

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

