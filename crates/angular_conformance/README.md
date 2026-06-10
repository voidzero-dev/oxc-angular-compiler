# Angular Conformance Testing

A conformance testing framework that validates the oxc Angular compiler implementation against Angular's official TypeScript compiler test specifications.

## Overview

This crate extracts test cases from Angular's TypeScript spec files and runs them against the Rust implementation to ensure compatibility. It currently maintains a **100% pass rate** across **1,264** extracted test assertions against Angular **v22.0.0-rc.2**.

## Architecture

```
Angular Spec Files (.ts)
        │
        ▼ (--generate)
   JSON Fixtures
        │
        ▼ (run)
  Subsystem Runners ──► Test Results ──► Markdown Report
```

### Components

1. **Spec Extractor** (`src/extractor/`) - Parses Angular's TypeScript spec files using oxc_parser and extracts test patterns into portable JSON fixtures.

2. **Subsystem Runners** (`src/subsystems/`) - Execute tests against specific compiler components:
   - `ExpressionParserRunner` - Action/binding expression parsing
   - `ExpressionLexerRunner` - Expression tokenization
   - `ExpressionSerializerRunner` - AST to string conversion
   - `HtmlParserRunner` - HTML template parsing with DOM humanization
   - `HtmlLexerRunner` - HTML tokenization
   - `HtmlWhitespaceRunner` - Whitespace normalization
   - `R3TransformRunner` - Ivy template transformation
   - `AstSpansRunner` - Source span tracking
   - `AstSerializerRunner` - Node serialization
   - `StyleParserRunner` - Inline style parsing
   - `ShadowCssRunner` - CSS scoping/shimming

3. **Report Generator** (`src/report.rs`) - Produces markdown reports with pass/fail summaries and detailed failure information.

## Usage

### Generate Fixtures

First, extract test fixtures from Angular's TypeScript specs:

```bash
cargo run -p oxc_angular_conformance -- --generate
```

This parses spec files from `crates/oxc_angular_compiler/angular/packages/compiler/test/` and writes JSON fixtures to `crates/angular_conformance/fixtures/`.

### Run Tests

Execute all conformance tests:

```bash
cargo run -p oxc_angular_conformance
```

### Filter Tests

Run tests matching a specific pattern:

```bash
cargo run -p oxc_angular_conformance -- --filter "parseAction"
cargo run -p oxc_angular_conformance -- --filter "bound"
```

### Debug Mode

Enable verbose output showing each test result:

```bash
cargo run -p oxc_angular_conformance -- --debug
cargo run -p oxc_angular_conformance -- --debug --filter "interpolation"
```

## Output

Test results are written to `crates/angular_conformance/snapshots/angular.snap.md` containing:

- Summary table with pass/fail counts per subsystem
- Overall pass rate percentage
- Detailed failure information (if any) with expected vs actual output

## Adding a New Subsystem Runner

1. Create a new file in `src/subsystems/` (e.g., `my_subsystem.rs`)

2. Implement the `SubsystemRunner` trait:

```rust
use crate::test_case::{TestAssertion, TestResult};
use crate::subsystems::SubsystemRunner;

pub struct MySubsystemRunner;

impl MySubsystemRunner {
    pub fn new() -> Self {
        Self
    }
}

impl SubsystemRunner for MySubsystemRunner {
    fn name(&self) -> &'static str {
        "my_subsystem"
    }

    fn description(&self) -> &'static str {
        "Tests for my subsystem functionality"
    }

    fn can_handle(&self, assertion: &TestAssertion) -> bool {
        matches!(assertion, TestAssertion::MyAssertionType { .. })
    }

    fn run_assertion(&self, assertion: &TestAssertion) -> TestResult {
        match assertion {
            TestAssertion::MyAssertionType { input, expected } => {
                // Run the test and compare results
                let actual = my_subsystem::process(input);
                if actual == *expected {
                    TestResult::Passed
                } else {
                    TestResult::Failed {
                        expected: expected.clone(),
                        actual,
                        diff: None,
                    }
                }
            }
            _ => TestResult::Skipped { reason: "Not handled".to_string() },
        }
    }
}
```

3. Register in `src/subsystems/mod.rs`:

```rust
mod my_subsystem;
pub use my_subsystem::MySubsystemRunner;

// In SubsystemRunners::new():
Box::new(MySubsystemRunner::new()),
```

4. If needed, add a new `TestAssertion` variant in `src/test_case.rs` and extraction logic in `src/extractor/`.

## Fixture Format

Fixtures are JSON files with this structure:

```json
{
  "name": "parser_spec.ts",
  "file_path": "/path/to/spec.ts",
  "test_groups": [
    {
      "name": "parser",
      "groups": [
        {
          "name": "parseAction",
          "tests": [
            {
              "name": "should parse numbers",
              "path": "parser/parseAction/should parse numbers",
              "assertions": [
                {
                  "type": "CheckAction",
                  "input": "1",
                  "expected": null
                }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

## Known Limitations

- Some spec files have 0 extracted assertions due to unsupported test patterns
- i18n and output generation subsystems are not yet implemented
- Parallel test execution is commented out (can be enabled with rayon)
