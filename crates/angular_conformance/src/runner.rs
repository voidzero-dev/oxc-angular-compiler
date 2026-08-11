//! Conformance test runner for Angular template testing.
//!
//! This module provides the [`ConformanceRunner`] which orchestrates the entire
//! conformance testing workflow:
//!
//! 1. **Fixture Generation** (`--generate` mode) - Extracts test cases from Angular's
//!    TypeScript spec files and serializes them to JSON fixtures.
//!
//! 2. **Test Execution** (default mode) - Loads fixtures, runs them through subsystem
//!    runners, and produces summary reports.
//!
//! ## Flow
//!
//! ```text
//! Angular Spec Files (.ts)
//!         │
//!         ▼ (--generate)
//!    JSON Fixtures
//!         │
//!         ▼ (run)
//!   Subsystem Runners ──► Test Results ──► Report
//! ```

use std::fs;
use std::path::{Path, PathBuf};

// use rayon::prelude::*; // TODO: Enable for parallel execution

use crate::extractor::SpecExtractor;
use crate::report::ReportGenerator;
use crate::subsystems::{SubsystemRunner, SubsystemRunners};
use crate::test_case::{TestCase, TestGroup, TestSuite, TestSummary};
use crate::{ConformanceOptions, angular_test_root, fixtures_root, snapshots_root};

/// Main conformance test runner that orchestrates fixture generation and test execution.
pub struct ConformanceRunner {
    options: ConformanceOptions,
    runners: SubsystemRunners,
}

impl ConformanceRunner {
    pub fn new(options: ConformanceOptions) -> Self {
        Self { options, runners: SubsystemRunners::new() }
    }

    /// Run conformance tests
    pub fn run(&self) {
        if self.options.generate {
            self.generate_fixtures();
        } else {
            self.run_tests();
        }
    }

    /// Generate fixtures from TypeScript spec files
    fn generate_fixtures(&self) {
        println!("Generating fixtures from Angular TypeScript specs...\n");

        let specs = self.discover_specs();
        println!("Found {} spec files\n", specs.len());

        let fixtures_dir = fixtures_root();
        fs::create_dir_all(&fixtures_dir).expect("Failed to create fixtures directory");

        for (spec_name, spec_path) in &specs {
            println!("Extracting: {spec_name}");

            let mut extractor = SpecExtractor::new();
            let suite = extractor.extract(spec_path);

            // Count tests
            let test_count = count_tests_in_suite(&suite);
            println!("  Found {test_count} test cases");

            // Write to JSON file
            let fixture_path = fixtures_dir.join(format!("{spec_name}.json"));
            let json = serde_json::to_string_pretty(&suite).expect("Failed to serialize");
            fs::write(&fixture_path, format!("{json}\n")).expect("Failed to write fixture");

            if self.options.debug {
                println!("  Written to: {}", fixture_path.display());
            }
        }

        println!("\nFixtures generated successfully!");
    }

    /// Run conformance tests from fixtures
    fn run_tests(&self) {
        println!("Running Angular conformance tests...\n");

        // Load fixtures
        let fixtures = self.load_fixtures();
        if fixtures.is_empty() {
            println!("No fixtures found. Run with --generate first.");
            return;
        }

        println!("Loaded {} fixture files\n", fixtures.len());

        // Collect all test cases
        let all_tests: Vec<TestCase> = fixtures.iter().flat_map(collect_tests_from_suite).collect();

        println!("Total test cases: {}\n", all_tests.len());

        // Filter tests if filter is specified
        let filtered_tests: Vec<&TestCase> = if let Some(filter) = &self.options.filter {
            all_tests
                .iter()
                .filter(|t| t.path.contains(filter) || t.name.contains(filter))
                .collect()
        } else {
            all_tests.iter().collect()
        };

        println!("Running {} tests (after filter)\n", filtered_tests.len());

        // Run tests for each subsystem
        let mut summaries: Vec<(&str, TestSummary)> = Vec::new();

        for runner in &self.runners.runners {
            let summary = self.run_subsystem_tests(runner.as_ref(), &filtered_tests);
            summaries.push((runner.name(), summary));
        }

        // Generate report
        let reporter = ReportGenerator::new();
        reporter.print_summary(&summaries);

        // Write snapshot
        let snapshot_path = snapshots_root().join("angular.snap.md");
        reporter.write_report(&snapshot_path, &summaries).expect("Failed to write snapshot");

        println!("Snapshot written to: {}", snapshot_path.display());
    }

    /// Run tests for a specific subsystem
    fn run_subsystem_tests(
        &self,
        runner: &dyn SubsystemRunner,
        tests: &[&TestCase],
    ) -> TestSummary {
        let mut summary = TestSummary::default();

        for test in tests {
            // Skip tests with no assertions this runner can handle
            let relevant_assertions: Vec<_> =
                test.assertions.iter().filter(|a| runner.can_handle(a)).collect();

            if relevant_assertions.is_empty() {
                continue;
            }

            for assertion in relevant_assertions {
                let result = runner.run_assertion(assertion);

                if self.options.debug {
                    println!(
                        "[{}] {}: {:?}",
                        runner.name(),
                        test.name,
                        if result.is_passed() { "PASS" } else { "FAIL" }
                    );
                }

                let assertion_name = format!("{}: {:?}", test.name, assertion);
                summary.add_result(&assertion_name, &test.path, result);
            }
        }

        summary
    }

    /// Discover all spec files in the Angular test directory
    fn discover_specs(&self) -> Vec<(String, PathBuf)> {
        let test_root = angular_test_root();
        if !test_root.exists() {
            println!("Angular test directory not found: {}", test_root.display());
            println!("Make sure submodules are initialized.");
            return vec![];
        }

        let mut specs = Vec::new();

        // Directories to scan recursively
        let directories = [
            "expression_parser",
            "ml_parser",
            "render3",
            "i18n",
            "shadow_css",
            "schema",
            "selector",
            "output",
        ];

        for dir_name in &directories {
            let dir = test_root.join(dir_name);
            scan_spec_dir(&dir, dir_name, &mut specs);
        }

        // Also scan top-level spec files
        scan_spec_dir(&test_root, "", &mut specs);

        specs
    }

    /// Load all fixture files
    fn load_fixtures(&self) -> Vec<TestSuite> {
        let fixtures_dir = fixtures_root();
        if !fixtures_dir.exists() {
            return vec![];
        }

        let mut fixtures = Vec::new();

        let entries = fs::read_dir(&fixtures_dir)
            .expect("Failed to read fixtures directory - check permissions");

        for entry in entries {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            if path.extension().is_some_and(|e| e == "json") {
                let content = fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("Failed to read fixture file {}: {}", path.display(), e)
                });
                match serde_json::from_str::<TestSuite>(&content) {
                    Ok(suite) => fixtures.push(suite),
                    Err(e) => {
                        eprintln!("Failed to parse {}: {}", path.display(), e);
                    }
                }
            }
        }

        fixtures
    }
}

/// Count total number of tests in a test suite
fn count_tests_in_suite(suite: &TestSuite) -> usize {
    suite.test_groups.iter().map(count_tests_in_group).sum()
}

/// Count tests in a test group (recursive)
fn count_tests_in_group(group: &TestGroup) -> usize {
    let own_tests = group.tests.len();
    let nested_tests: usize = group.groups.iter().map(count_tests_in_group).sum();
    own_tests + nested_tests
}

/// Collect all test cases from a test suite
fn collect_tests_from_suite(suite: &TestSuite) -> Vec<TestCase> {
    suite.test_groups.iter().flat_map(collect_tests_from_group).collect()
}

/// Collect test cases from a group (recursive)
fn collect_tests_from_group(group: &TestGroup) -> Vec<TestCase> {
    let mut tests = group.tests.clone();
    for nested in &group.groups {
        tests.extend(collect_tests_from_group(nested));
    }
    tests
}

/// Recursively scan a directory for spec files
fn scan_spec_dir(dir: &Path, prefix: &str, specs: &mut Vec<(String, PathBuf)>) {
    if !dir.exists() {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else { return };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            // Recursively scan subdirectories
            let subdir_name = path.file_name().unwrap().to_string_lossy();
            let new_prefix = if prefix.is_empty() {
                subdir_name.to_string()
            } else {
                format!("{prefix}_{subdir_name}")
            };
            scan_spec_dir(&path, &new_prefix, specs);
        } else if path.extension().is_some_and(|e| e == "ts") {
            let file_name = path.file_name().unwrap().to_string_lossy();
            if file_name.ends_with("_spec.ts") {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                let fixture_name =
                    if prefix.is_empty() { name } else { format!("{prefix}_{name}") };
                specs.push((fixture_name, path));
            }
        }
    }
}

impl Default for ConformanceRunner {
    fn default() -> Self {
        Self::new(ConformanceOptions::default())
    }
}
