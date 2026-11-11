# Angular Conformance Test Results

## Summary

| Subsystem | Passed | Failed | Errors | Skipped | Total | Pass Rate |
|-----------|--------|--------|--------|---------|-------|-----------|
| ast_serializer | 6 | 0 | 0 | 0 | 6 | 100.0% |
| ast_spans | 55 | 0 | 0 | 0 | 55 | 100.0% |
| expression_lexer | 138 | 16 | 0 | 0 | 154 | 89.6% |
| expression_parser | 275 | 58 | 70 | 0 | 403 | 68.2% |
| expression_serializer | 38 | 0 | 6 | 0 | 44 | 86.4% |
| html_lexer | 291 | 6 | 0 | 0 | 297 | 98.0% |
| html_parser | 89 | 1 | 0 | 0 | 90 | 98.9% |
| html_whitespace | 21 | 0 | 0 | 0 | 21 | 100.0% |
| r3_transform | 180 | 4 | 0 | 0 | 184 | 97.8% |
| shadow_css | 185 | 2 | 0 | 0 | 187 | 98.9% |
| style_parser | 15 | 0 | 0 | 0 | 15 | 100.0% |
| **Total** | **1293** | **87** | **76** | **0** | **1456** | **88.8%** |

## Failed Tests

### expression_lexer

#### should tokenize increment/decrement operators: ExpressionLexerTest { input: "++", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 2, value: Some(String("++")) }] }
Path: `lexer/token/should tokenize increment/decrement operators`

**Expected:**
```
Token type Operator at index 0
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators: ExpressionLexerTest { input: "--", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 2, value: Some(String("--")) }] }
Path: `lexer/token/should tokenize increment/decrement operators`

**Expected:**
```
Token type Operator at index 0
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators: ExpressionLexerTest { input: "a++", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 3, value: Some(String("++")) }] }
Path: `lexer/token/should tokenize increment/decrement operators`

**Expected:**
```
Token type Operator at index 1
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators: ExpressionLexerTest { input: "a--", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 3, value: Some(String("--")) }] }
Path: `lexer/token/should tokenize increment/decrement operators`

**Expected:**
```
Token type Operator at index 1
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators: ExpressionLexerTest { input: "++a", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 2, value: Some(String("++")) }] }
Path: `lexer/token/should tokenize increment/decrement operators`

**Expected:**
```
Token type Operator at index 0
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators: ExpressionLexerTest { input: "--a", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 2, value: Some(String("--")) }] }
Path: `lexer/token/should tokenize increment/decrement operators`

**Expected:**
```
Token type Operator at index 0
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators used together with plus/minus: ExpressionLexerTest { input: "+++a", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 2, value: Some(String("++")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 2, end: 3, value: Some(String("+")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 2, start: 3, end: 4, value: Some(String("a")) }] }
Path: `lexer/token/should tokenize increment/decrement operators used together with plus/minus`

**Expected:**
```
Token type Operator at index 0
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators used together with plus/minus: ExpressionLexerTest { input: "---a", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 2, value: Some(String("--")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 2, end: 3, value: Some(String("-")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 2, start: 3, end: 4, value: Some(String("a")) }] }
Path: `lexer/token/should tokenize increment/decrement operators used together with plus/minus`

**Expected:**
```
Token type Operator at index 0
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators used together with plus/minus: ExpressionLexerTest { input: "-- foo++", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 2, value: Some(String("--")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 1, start: 3, end: 6, value: Some(String("foo")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 2, start: 6, end: 8, value: Some(String("++")) }] }
Path: `lexer/token/should tokenize increment/decrement operators used together with plus/minus`

**Expected:**
```
Token type Operator at index 0
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators used together with plus/minus: ExpressionLexerTest { input: "- - foo++", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Operator, token_index: 0, start: 0, end: 1, value: Some(String("-")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 2, end: 3, value: Some(String("-")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 2, start: 4, end: 7, value: Some(String("foo")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 3, start: 7, end: 9, value: Some(String("++")) }] }
Path: `lexer/token/should tokenize increment/decrement operators used together with plus/minus`

**Expected:**
```
Token type Operator at index 3
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators followed by other operators in an expression: ExpressionLexerTest { input: "a+++b", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Identifier, token_index: 0, start: 0, end: 1, value: Some(String("a")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 3, value: Some(String("++")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 2, start: 3, end: 4, value: Some(String("+")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 3, start: 4, end: 5, value: Some(String("b")) }] }
Path: `lexer/token/should tokenize increment/decrement operators followed by other operators in an expression`

**Expected:**
```
Token type Operator at index 1
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators followed by other operators in an expression: ExpressionLexerTest { input: "a+++++b", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Identifier, token_index: 0, start: 0, end: 1, value: Some(String("a")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 3, value: Some(String("++")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 2, start: 3, end: 5, value: Some(String("++")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 3, start: 5, end: 6, value: Some(String("+")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 4, start: 6, end: 7, value: Some(String("b")) }] }
Path: `lexer/token/should tokenize increment/decrement operators followed by other operators in an expression`

**Expected:**
```
Token type Operator at index 1
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators followed by other operators in an expression: ExpressionLexerTest { input: "a-----b", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Identifier, token_index: 0, start: 0, end: 1, value: Some(String("a")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 3, value: Some(String("--")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 2, start: 3, end: 5, value: Some(String("--")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 3, start: 5, end: 6, value: Some(String("-")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 4, start: 6, end: 7, value: Some(String("b")) }] }
Path: `lexer/token/should tokenize increment/decrement operators followed by other operators in an expression`

**Expected:**
```
Token type Operator at index 1
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators followed by other operators in an expression: ExpressionLexerTest { input: "a+++ +b", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Identifier, token_index: 0, start: 0, end: 1, value: Some(String("a")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 3, value: Some(String("++")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 2, start: 3, end: 4, value: Some(String("+")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 3, start: 5, end: 6, value: Some(String("+")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 4, start: 6, end: 7, value: Some(String("b")) }] }
Path: `lexer/token/should tokenize increment/decrement operators followed by other operators in an expression`

**Expected:**
```
Token type Operator at index 1
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators followed by other operators in an expression: ExpressionLexerTest { input: "a+--b", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Identifier, token_index: 0, start: 0, end: 1, value: Some(String("a")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 2, value: Some(String("+")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 2, start: 2, end: 4, value: Some(String("--")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 3, start: 4, end: 5, value: Some(String("b")) }] }
Path: `lexer/token/should tokenize increment/decrement operators followed by other operators in an expression`

**Expected:**
```
Token type Operator at index 2
```

**Actual:**
```
Token type Error
```

#### should tokenize increment/decrement operators followed by other operators in an expression: ExpressionLexerTest { input: "a+++=b", expected_token_count: None, token_assertions: [ExpressionTokenAssertion { assertion_type: Identifier, token_index: 0, start: 0, end: 1, value: Some(String("a")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 1, start: 1, end: 3, value: Some(String("++")) }, ExpressionTokenAssertion { assertion_type: Operator, token_index: 2, start: 3, end: 5, value: Some(String("+=")) }, ExpressionTokenAssertion { assertion_type: Identifier, token_index: 3, start: 5, end: 6, value: Some(String("b")) }] }
Path: `lexer/token/should tokenize increment/decrement operators followed by other operators in an expression`

**Expected:**
```
Token type Operator at index 1
```

**Actual:**
```
Token type Error
```

### expression_parser

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "a++ ** 2", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ ** 2]"]
```

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "++a ** 2", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a ** 2]"]
```

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "a-- ** 2", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 2 in [a-- ** 2]"]
```

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "--a ** 2", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--a ** 2]"]
```

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "a.b++ ** 2", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [a.b++ ** 2]"]
```

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "a[0]++ ** 2", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 5 in [a[0]++ ** 2]"]
```

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "2 ** a++", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [2 ** a++]"]
```

#### should allow ++ and -- on the left-hand side of **: CheckAction { input: "a ** b++ ** c", expected: None }
Path: `parser/parseAction/should allow ++ and -- on the left-hand side of **`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [a ** b++ ** c]"]
```

#### should parse prefix ++ and --: CheckAction { input: "++a", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a]"]
```

#### should parse prefix ++ and --: CheckAction { input: "--a", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--a]"]
```

#### should parse prefix ++ and --: CheckAction { input: "++a.b", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a.b]"]
```

#### should parse prefix ++ and --: CheckAction { input: "--a.b", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--a.b]"]
```

#### should parse prefix ++ and --: CheckAction { input: "++a[0]", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a[0]]"]
```

#### should parse prefix ++ and --: CheckAction { input: "--a[0]", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--a[0]]"]
```

#### should parse prefix ++ and --: CheckAction { input: "++this.a", expected: Some("++a") }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++this.a]"]
```

#### should parse prefix ++ and --: CheckAction { input: "--this.a", expected: Some("--a") }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--this.a]"]
```

#### should parse prefix ++ and --: CheckAction { input: "++a!", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a!]"]
```

#### should parse prefix ++ and --: CheckAction { input: "--a!", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--a!]"]
```

#### should parse prefix ++ and --: CheckAction { input: "++(a)", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++(a)]"]
```

#### should parse prefix ++ and --: CheckAction { input: "--(a)", expected: None }
Path: `parser/parseAction/should parse prefix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--(a)]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a++", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a--", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 2 in [a--]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a.b++", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [a.b++]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a.b--", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 4 in [a.b--]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a[0]++", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 5 in [a[0]++]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a[0]--", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 5 in [a[0]--]"]
```

#### should parse postfix ++ and --: CheckAction { input: "this.a++", expected: Some("a++") }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [this.a++]"]
```

#### should parse postfix ++ and --: CheckAction { input: "this.a--", expected: Some("a--") }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 7 in [this.a--]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a!++", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 3 in [a!++]"]
```

#### should parse postfix ++ and --: CheckAction { input: "a!--", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 3 in [a!--]"]
```

#### should parse postfix ++ and --: CheckAction { input: "(a)++", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [(a)++]"]
```

#### should parse postfix ++ and --: CheckAction { input: "(a)--", expected: None }
Path: `parser/parseAction/should parse postfix ++ and --`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 4 in [(a)--]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "- - foo++", expected: Some("- -foo++") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 8 in [- - foo++]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "+ + foo++", expected: Some("+ +foo++") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 8 in [+ + foo++]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "- ++foo", expected: Some("-++foo") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 3 in [- ++foo]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "+ --foo", expected: Some("+--foo") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 3 in [+ --foo]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "a+++b", expected: Some("a++ + b") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a+++b]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "a---b", expected: Some("a-- - b") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "-- is not allowed", "Unexpected token -- is not allowed at column 2 in [a---b]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "a+++b+++c", expected: Some("a++ + b++ + c") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "++ is not allowed", "++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a+++b+++c]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "a+++ +b", expected: Some("a++ + +b") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a+++ +b]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "a+ ++b", expected: Some("a + ++b") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [a+ ++b]"]
```

#### should parse consecutive unary expressions: CheckAction { input: "a- --b", expected: Some("a - --b") }
Path: `parser/parseAction/should parse consecutive unary expressions`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 4 in [a- --b]"]
```

#### should bind ++ and -- tighter than the other unary operators: CheckAction { input: "-a++", expected: None }
Path: `parser/parseAction/should bind ++ and -- tighter than the other unary operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 3 in [-a++]"]
```

#### should bind ++ and -- tighter than the other unary operators: CheckAction { input: "+a++", expected: None }
Path: `parser/parseAction/should bind ++ and -- tighter than the other unary operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 3 in [+a++]"]
```

#### should bind ++ and -- tighter than the other unary operators: CheckAction { input: "!a++", expected: None }
Path: `parser/parseAction/should bind ++ and -- tighter than the other unary operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 3 in [!a++]"]
```

#### should bind ++ and -- tighter than the other unary operators: CheckAction { input: "-a--", expected: None }
Path: `parser/parseAction/should bind ++ and -- tighter than the other unary operators`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 3 in [-a--]"]
```

#### should bind ++ and -- tighter than the other unary operators: CheckAction { input: "typeof a++", expected: None }
Path: `parser/parseAction/should bind ++ and -- tighter than the other unary operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 9 in [typeof a++]"]
```

#### should bind ++ and -- tighter than the other unary operators: CheckAction { input: "void a++", expected: None }
Path: `parser/parseAction/should bind ++ and -- tighter than the other unary operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [void a++]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a++ instanceof b", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ instanceof b]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a++ in o", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ in o]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a++ < 3", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ < 3]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a++ * 2", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ * 2]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a++ ?? b", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ ?? b]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a ? b++ : c--", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "-- is not allowed", "Conditional expression a ? b requires all 3 expressions at column 6"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "o[a++]", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Missing expected ] at column 4"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "f(a++)", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Missing expected ) at column 4"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "[a++]", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Missing expected ] at column 3"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "{x: a++}", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Missing expected } at column 6"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "c = a++", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 6 in [c = a++]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "c += a++", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [c += a++]"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a.b[c++].d++", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "++ is not allowed", "Missing expected ] at column 6"]
```

#### should allow ++ and -- expressions as operands of other operators: CheckAction { input: "a++ + ++a", expected: None }
Path: `parser/parseAction/should allow ++ and -- expressions as operands of other operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ + ++a]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "-- foo++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["-- is not allowed", "++ is not allowed", "Unexpected token -- is not allowed at column 1 in [-- foo++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++ foo--", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "-- is not allowed", "Unexpected token ++ is not allowed at column 1 in [++ foo--]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "a?.a++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 5 in [a?.a++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++a?.a", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a?.a]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "a?.[0]++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [a?.[0]++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++a?.[0]", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a?.[0]]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "5++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [5++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++5", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++5]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "foo()++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 6 in [foo()++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++foo()", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++foo()]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "---foo", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["-- is not allowed", "-- is not allowed", "Unexpected token -- is not allowed at column 1 in [---foo]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "+++foo", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [+++foo]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "(foo++)++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Missing closing parentheses at column 5", "Unexpected token ++ is not allowed at column 8 in [(foo++)++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++(foo++)", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++(foo++)]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++foo++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++foo++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "--foo--", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["-- is not allowed", "-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--foo--]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++foo--", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "-- is not allowed", "Unexpected token ++ is not allowed at column 1 in [++foo--]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "--foo++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["-- is not allowed", "++ is not allowed", "Unexpected token -- is not allowed at column 1 in [--foo++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++ ++foo", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++ ++foo]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "-- --foo", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["-- is not allowed", "-- is not allowed", "Unexpected token -- is not allowed at column 1 in [-- --foo]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++foo+++bar", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++foo+++bar]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "(-foo)++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [(-foo)++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "(foo + bar)++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 12 in [(foo + bar)++]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "++(foo + bar)", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++(foo + bar)]"]
```

#### should report invalid increment/decrement expressions: ExpectActionError { input: "\"foo\"++", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should report invalid increment/decrement expressions`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 6 in [\"foo\"++]"]
```

#### should report unexpected token for chained postfix operators: ExpectActionError { input: "foo++ ++", error_contains: "Unexpected token '++'" }
Path: `parser/parseAction/assignment/should report unexpected token for chained postfix operators`

**Expected:**
```
Error containing: Unexpected token '++'
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [foo++ ++]"]
```

#### should report unexpected token for chained postfix operators: ExpectActionError { input: "foo-- --", error_contains: "Unexpected token '--'" }
Path: `parser/parseAction/assignment/should report unexpected token for chained postfix operators`

**Expected:**
```
Error containing: Unexpected token '--'
```

**Actual:**
```
Errors: ["-- is not allowed", "-- is not allowed", "Unexpected token -- is not allowed at column 4 in [foo-- --]"]
```

#### should report unexpected token for chained postfix operators: ExpectActionError { input: "foo+++++bar", error_contains: "Unexpected token '++'" }
Path: `parser/parseAction/assignment/should report unexpected token for chained postfix operators`

**Expected:**
```
Error containing: Unexpected token '++'
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [foo+++++bar]"]
```

#### should report unexpected token for chained postfix operators: ExpectActionError { input: "foo-----bar", error_contains: "Unexpected token '--'" }
Path: `parser/parseAction/assignment/should report unexpected token for chained postfix operators`

**Expected:**
```
Error containing: Unexpected token '--'
```

**Actual:**
```
Errors: ["-- is not allowed", "-- is not allowed", "-- is not allowed", "-- is not allowed", "Unexpected token -- is not allowed at column 4 in [foo-----bar]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "a+++", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a+++]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "a---", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["-- is not allowed", "-- is not allowed", "Unexpected token -- is not allowed at column 2 in [a---]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "a++ +", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++ +]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "a++-", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a++-]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "++a+", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++a+]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "a+++b+", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [a+++b+]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "foo.bar+++", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 8 in [foo.bar+++]"]
```

#### should report a dangling + or - left over after an update operator: ExpectActionError { input: "foo[0]+++", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a dangling + or - left over after an update operator`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 7 in [foo[0]+++]"]
```

#### should report a increment/decrement expression with no target: ExpectActionError { input: "++", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a increment/decrement expression with no target`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++]"]
```

#### should report a increment/decrement expression with no target: ExpectActionError { input: "--", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a increment/decrement expression with no target`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 1 in [--]"]
```

#### should report a increment/decrement expression with no target: ExpectActionError { input: "+++", error_contains: "Unexpected end of expression" }
Path: `parser/parseAction/assignment/should report a increment/decrement expression with no target`

**Expected:**
```
Error containing: Unexpected end of expression
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [+++]"]
```

#### should not allow member access or calls after a postfix update: ExpectActionError { input: "foo++.bar", error_contains: "Unexpected token '.'" }
Path: `parser/parseAction/assignment/should not allow member access or calls after a postfix update`

**Expected:**
```
Error containing: Unexpected token '.'
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [foo++.bar]"]
```

#### should not allow member access or calls after a postfix update: ExpectActionError { input: "foo++[0]", error_contains: "Unexpected token '['" }
Path: `parser/parseAction/assignment/should not allow member access or calls after a postfix update`

**Expected:**
```
Error containing: Unexpected token '['
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [foo++[0]]"]
```

#### should not allow member access or calls after a postfix update: ExpectActionError { input: "foo++()", error_contains: "Unexpected token '('" }
Path: `parser/parseAction/assignment/should not allow member access or calls after a postfix update`

**Expected:**
```
Error containing: Unexpected token '('
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [foo++()]"]
```

#### should not allow member access or calls after a postfix update: ExpectActionError { input: "foo--.bar", error_contains: "Unexpected token '.'" }
Path: `parser/parseAction/assignment/should not allow member access or calls after a postfix update`

**Expected:**
```
Error containing: Unexpected token '.'
```

**Actual:**
```
Errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 4 in [foo--.bar]"]
```

#### should not allow member access or calls after a postfix update: ExpectActionError { input: "foo-- bar", error_contains: "Unexpected token 'bar'" }
Path: `parser/parseAction/assignment/should not allow member access or calls after a postfix update`

**Expected:**
```
Error containing: Unexpected token 'bar'
```

**Actual:**
```
Errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 4 in [foo-- bar]"]
```

#### should not allow assigning to an update expression: ExpectActionError { input: "foo++ = 1", error_contains: "Unexpected token '='" }
Path: `parser/parseAction/assignment/should not allow assigning to an update expression`

**Expected:**
```
Error containing: Unexpected token '='
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [foo++ = 1]"]
```

#### should not allow assigning to an update expression: ExpectActionError { input: "++foo = 1", error_contains: "cannot be used in the assignment" }
Path: `parser/parseAction/assignment/should not allow assigning to an update expression`

**Expected:**
```
Error containing: cannot be used in the assignment
```

**Actual:**
```
Errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 1 in [++foo = 1]"]
```

#### should not allow assigning to an update expression: ExpectActionError { input: "foo+++=bar", error_contains: "Unexpected token '+='" }
Path: `parser/parseAction/assignment/should not allow assigning to an update expression`

**Expected:**
```
Error containing: Unexpected token '+='
```

**Actual:**
```
Errors: ["++ is not allowed", "++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [foo+++=bar]"]
```

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
Errors: ["Unexpected token } at column 14 in [`hello ${a; b} world`]"]
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

#### should parse an arrow function containing update operators: CheckBinding { input: "a => a++", expected: None }
Path: `parser/parseBinding/arrow functions/should parse an arrow function containing update operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token '++ is not allowed' at column 7 in [a => a++]"]
```

#### should parse an arrow function containing update operators: CheckBinding { input: "a => a--", expected: None }
Path: `parser/parseBinding/arrow functions/should parse an arrow function containing update operators`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token '-- is not allowed' at column 7 in [a => a--]"]
```

#### should parse an arrow function containing update operators: CheckBinding { input: "a => ++a", expected: None }
Path: `parser/parseBinding/arrow functions/should parse an arrow function containing update operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token '++ is not allowed' at column 6 in [a => ++a]"]
```

#### should parse an arrow function containing update operators: CheckBinding { input: "a => --a", expected: None }
Path: `parser/parseBinding/arrow functions/should parse an arrow function containing update operators`

**Error:**
```
Unexpected parse errors: ["-- is not allowed", "Unexpected token '-- is not allowed' at column 6 in [a => --a]"]
```

#### should parse an arrow function containing update operators: CheckBinding { input: "(a) => a++", expected: Some("a => a++") }
Path: `parser/parseBinding/arrow functions/should parse an arrow function containing update operators`

**Error:**
```
Unexpected parse errors: ["++ is not allowed", "Unexpected token '++ is not allowed' at column 9 in [(a) => a++]"]
```

### expression_serializer

#### should serialize prefix increment and decrement: SerializeExpression { input: " ++ foo ", expected: "++foo" }
Path: `serializer/serialize/should serialize prefix increment and decrement`

**Error:**
```
Parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 2 in [ ++ foo ]"]
```

#### should serialize prefix increment and decrement: SerializeExpression { input: " -- foo ", expected: "--foo" }
Path: `serializer/serialize/should serialize prefix increment and decrement`

**Error:**
```
Parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 2 in [ -- foo ]"]
```

#### should serialize postfix increment and decrement: SerializeExpression { input: " foo ++ ", expected: "foo++" }
Path: `serializer/serialize/should serialize postfix increment and decrement`

**Error:**
```
Parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 6 in [ foo ++ ]"]
```

#### should serialize postfix increment and decrement: SerializeExpression { input: " foo -- ", expected: "foo--" }
Path: `serializer/serialize/should serialize postfix increment and decrement`

**Error:**
```
Parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 6 in [ foo -- ]"]
```

#### should separate unary operators next to each other: SerializeExpression { input: " + ++foo ", expected: "+ ++foo" }
Path: `serializer/serialize/should separate unary operators next to each other`

**Error:**
```
Parse errors: ["++ is not allowed", "Unexpected token ++ is not allowed at column 4 in [ + ++foo ]"]
```

#### should separate unary operators next to each other: SerializeExpression { input: " - --foo ", expected: "- --foo" }
Path: `serializer/serialize/should separate unary operators next to each other`

**Error:**
```
Parse errors: ["-- is not allowed", "Unexpected token -- is not allowed at column 4 in [ - --foo ]"]
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

