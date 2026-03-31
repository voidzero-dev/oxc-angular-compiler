use oxc_allocator::{Allocator, FromIn};
use oxc_span::Ident;
use rustc_hash::FxHashMap;

/// Registry for assigning namespace aliases to imported modules.
///
/// Angular uses namespace aliases like i0, i1, i2... for imported modules:
/// - i0 is always @angular/core
/// - i1, i2, i3... are assigned to other modules in order of first reference
pub struct NamespaceRegistry<'a> {
    /// Map of module_path -> namespace alias
    modules: FxHashMap<Ident<'a>, Ident<'a>>,
    /// Counter for next alias index (starts at 1, since i0 is reserved for @angular/core)
    next_index: usize,
    /// The allocator for creating atoms
    allocator: &'a Allocator,
}

impl<'a> NamespaceRegistry<'a> {
    /// Creates a new namespace registry.
    ///
    /// The registry is initialized with `@angular/core` pre-registered as `i0`.
    pub fn new(allocator: &'a Allocator) -> Self {
        let mut registry = Self {
            modules: FxHashMap::default(),
            next_index: 1, // Start at 1 (i0 is reserved)
            allocator,
        };
        // Pre-register @angular/core as i0
        registry.modules.insert(Ident::from("@angular/core"), Ident::from("i0"));
        registry
    }

    /// Get the namespace alias for a module, assigning one if not yet assigned.
    pub fn get_or_assign(&mut self, module_path: &Ident<'a>) -> Ident<'a> {
        if let Some(alias) = self.modules.get(module_path) {
            return alias.clone();
        }

        // Assign new alias
        let alias = Ident::from_in(format!("i{}", self.next_index).as_str(), self.allocator);
        self.next_index += 1;
        self.modules.insert(module_path.clone(), alias.clone());
        alias
    }

    /// Get all registered modules and their aliases.
    /// Returns in a deterministic order (sorted by alias).
    pub fn get_all_modules(&self) -> Vec<(&Ident<'a>, &Ident<'a>)> {
        let mut entries: Vec<_> = self.modules.iter().collect();
        entries.sort_by_key(|(_, alias)| alias.as_str());
        entries
    }

    /// Check if a module has been registered.
    pub fn has_module(&self, module_path: &Ident<'a>) -> bool {
        self.modules.contains_key(module_path)
    }

    /// Generate import statements for all registered modules.
    ///
    /// Returns a string containing namespace import statements in sorted order:
    /// ```javascript
    /// import * as i0 from "@angular/core";
    /// import * as i1 from "@bitwarden/common/auth/abstractions/auth.service";
    /// import * as i2 from "@angular/router";
    /// ```
    pub fn generate_import_statements(&self) -> String {
        let modules = self.get_all_modules();
        let mut result = String::new();

        for (module_path, alias) in modules {
            result.push_str("import * as ");
            result.push_str(alias.as_str());
            result.push_str(" from '");
            result.push_str(module_path.as_str());
            result.push_str("';\n");
        }

        result
    }

    /// Merge another namespace registry into this one.
    ///
    /// Used to combine namespace registries from multiple components in the same file.
    /// Note: This assumes that modules already registered in both registries have
    /// the same alias (which should be true if they were registered in the same order
    /// from the @angular/core starting point).
    pub fn merge_from(&mut self, other: &Self) {
        for (module_path, alias) in &other.modules {
            if !self.modules.contains_key(module_path) {
                self.modules.insert(module_path.clone(), alias.clone());
                // Update next_index if the merged alias index is higher
                if let Some(idx_str) = alias.strip_prefix('i') {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        if idx >= self.next_index {
                            self.next_index = idx + 1;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_angular_core_is_always_i0() {
        let allocator = Allocator::default();
        let mut registry = NamespaceRegistry::new(&allocator);

        let core_alias = registry.get_or_assign(&Ident::from("@angular/core"));
        assert_eq!(core_alias.as_str(), "i0");
    }

    #[test]
    fn test_other_modules_get_sequential_aliases() {
        let allocator = Allocator::default();
        let mut registry = NamespaceRegistry::new(&allocator);

        let forms_alias = registry.get_or_assign(&Ident::from("@angular/forms"));
        let router_alias = registry.get_or_assign(&Ident::from("@angular/router"));
        let http_alias = registry.get_or_assign(&Ident::from("@angular/common/http"));

        assert_eq!(forms_alias.as_str(), "i1");
        assert_eq!(router_alias.as_str(), "i2");
        assert_eq!(http_alias.as_str(), "i3");
    }

    #[test]
    fn test_same_module_returns_same_alias() {
        let allocator = Allocator::default();
        let mut registry = NamespaceRegistry::new(&allocator);

        let first = registry.get_or_assign(&Ident::from("@angular/forms"));
        let second = registry.get_or_assign(&Ident::from("@angular/forms"));

        assert_eq!(first.as_str(), second.as_str());
        assert_eq!(first.as_str(), "i1");
    }

    #[test]
    fn test_has_module() {
        let allocator = Allocator::default();
        let mut registry = NamespaceRegistry::new(&allocator);

        // @angular/core is pre-registered
        assert!(registry.has_module(&Ident::from("@angular/core")));

        // Not yet registered
        assert!(!registry.has_module(&Ident::from("@angular/forms")));

        // Register it
        registry.get_or_assign(&Ident::from("@angular/forms"));
        assert!(registry.has_module(&Ident::from("@angular/forms")));
    }

    #[test]
    fn test_get_all_modules_sorted_by_alias() {
        let allocator = Allocator::default();
        let mut registry = NamespaceRegistry::new(&allocator);

        registry.get_or_assign(&Ident::from("@angular/router"));
        registry.get_or_assign(&Ident::from("@angular/forms"));

        let all_modules = registry.get_all_modules();

        // Should be sorted by alias: i0, i1, i2
        assert_eq!(all_modules.len(), 3);
        assert_eq!(all_modules[0].1.as_str(), "i0");
        assert_eq!(all_modules[0].0.as_str(), "@angular/core");
        assert_eq!(all_modules[1].1.as_str(), "i1");
        assert_eq!(all_modules[1].0.as_str(), "@angular/router");
        assert_eq!(all_modules[2].1.as_str(), "i2");
        assert_eq!(all_modules[2].0.as_str(), "@angular/forms");
    }

    #[test]
    fn test_angular_core_requested_later_still_returns_i0() {
        let allocator = Allocator::default();
        let mut registry = NamespaceRegistry::new(&allocator);

        // Register other modules first
        registry.get_or_assign(&Ident::from("@angular/forms"));
        registry.get_or_assign(&Ident::from("@angular/router"));

        // @angular/core should still be i0
        let core_alias = registry.get_or_assign(&Ident::from("@angular/core"));
        assert_eq!(core_alias.as_str(), "i0");

        // Next module should be i3
        let http_alias = registry.get_or_assign(&Ident::from("@angular/common/http"));
        assert_eq!(http_alias.as_str(), "i3");
    }

    #[test]
    fn test_generate_import_statements() {
        let allocator = Allocator::default();
        let mut registry = NamespaceRegistry::new(&allocator);

        registry.get_or_assign(&Ident::from("@angular/router"));
        registry.get_or_assign(&Ident::from("@bitwarden/common/auth/abstractions/auth.service"));

        let imports = registry.generate_import_statements();

        // Should be sorted by alias and formatted correctly
        let expected = "\
import * as i0 from '@angular/core';
import * as i1 from '@angular/router';
import * as i2 from '@bitwarden/common/auth/abstractions/auth.service';
";
        assert_eq!(imports, expected);
    }

    #[test]
    fn test_generate_import_statements_only_core() {
        let allocator = Allocator::default();
        let registry = NamespaceRegistry::new(&allocator);

        let imports = registry.generate_import_statements();

        assert_eq!(imports, "import * as i0 from '@angular/core';\n");
    }

    #[test]
    fn test_merge_from() {
        let allocator = Allocator::default();
        let mut registry1 = NamespaceRegistry::new(&allocator);
        let mut registry2 = NamespaceRegistry::new(&allocator);

        // Registry 1 has @angular/router as i1
        registry1.get_or_assign(&Ident::from("@angular/router"));

        // Registry 2 has @angular/forms as i1, @angular/common as i2
        registry2.get_or_assign(&Ident::from("@angular/forms"));
        registry2.get_or_assign(&Ident::from("@angular/common"));

        // Merge registry2 into registry1
        registry1.merge_from(&registry2);

        // registry1 should now have all modules
        assert!(registry1.has_module(&Ident::from("@angular/core")));
        assert!(registry1.has_module(&Ident::from("@angular/router")));
        assert!(registry1.has_module(&Ident::from("@angular/forms")));
        assert!(registry1.has_module(&Ident::from("@angular/common")));

        // New module should get next available index
        let http_alias = registry1.get_or_assign(&Ident::from("@angular/common/http"));
        assert_eq!(http_alias.as_str(), "i3");
    }
}
