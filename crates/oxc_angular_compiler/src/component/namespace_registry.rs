use oxc_allocator::{Allocator, FromIn};
use oxc_str::Ident;
use rustc_hash::{FxHashMap, FxHashSet};

/// Registry for assigning namespace aliases to imported modules.
///
/// Angular uses namespace aliases like i0, i1, i2... for imported modules:
/// - i0 is always @angular/core
/// - i1, i2, i3... are assigned to other modules in order of first reference
///
/// Like ngtsc's `ImportManager` (`import_manager.ts:229`), an alias colliding
/// with any identifier the file already declares is uniquified with a `_N`
/// suffix (`i0` → `i0_1`, ...), and an existing non-type-only
/// `import * as ns from "..."` for the module is reused instead of emitting a
/// second import (`reuse_source_file_imports.ts`).
pub struct NamespaceRegistry<'a> {
    /// Map of module_path -> namespace alias
    modules: FxHashMap<Ident<'a>, Ident<'a>>,
    /// Module paths whose alias names an import that already exists in the
    /// file — `generate_import_statements` must not re-emit them.
    reused_modules: FxHashSet<Ident<'a>>,
    /// Every identifier the source file already declares or references. Any
    /// newly assigned alias is uniquified against this set (and against
    /// aliases the registry itself assigned earlier).
    used_names: FxHashSet<String>,
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
        Self::with_file_scope(allocator, None, None)
    }

    /// Creates a namespace registry that is aware of the file's existing names.
    ///
    /// `reused_core_alias` is the local name of an existing
    /// `import * as ns from "@angular/core"` in the file; when provided, the
    /// registry references `@angular/core` through `ns` and does not emit its
    /// own import. Otherwise the `@angular/core` alias is `i0`, uniquified as
    /// `i0_1`, `i0_2`, ... while `used_names` contains it.
    pub fn with_file_scope(
        allocator: &'a Allocator,
        reused_core_alias: Option<Ident<'a>>,
        used_names: Option<FxHashSet<String>>,
    ) -> Self {
        let mut registry = Self {
            modules: FxHashMap::default(),
            reused_modules: FxHashSet::default(),
            used_names: used_names.unwrap_or_default(),
            next_index: 1, // Start at 1 (i0 is reserved)
            allocator,
        };
        // Pre-register @angular/core
        let core_module = Ident::from("@angular/core");
        let core_alias = match reused_core_alias {
            Some(alias) => {
                registry.reused_modules.insert(core_module);
                alias
            }
            None => registry.unique_name("i0"),
        };
        registry.modules.insert(core_module, core_alias);
        registry
    }

    /// Returns `name` unchanged when the file does not already use it, else
    /// `name_1`, `name_2`, ... — the same rule as ngtsc's
    /// `generateUniqueIdentifier` (`check_unique_identifier_name.ts`).
    fn unique_name(&mut self, name: &str) -> Ident<'a> {
        if !self.used_names.contains(name) {
            self.used_names.insert(name.to_string());
            return Ident::from_in(name, self.allocator);
        }
        let mut counter = 1;
        loop {
            let candidate = format!("{name}_{counter}");
            counter += 1;
            if !self.used_names.contains(&candidate) {
                self.used_names.insert(candidate.clone());
                return Ident::from_in(candidate, self.allocator);
            }
        }
    }

    /// The alias generated `@angular/core` references use (usually `i0`).
    pub fn angular_core_ns(&self) -> Ident<'a> {
        // @angular/core is always pre-registered by the constructors.
        self.modules[&Ident::from("@angular/core")]
    }

    /// Get the namespace alias for a module, assigning one if not yet assigned.
    pub fn get_or_assign(&mut self, module_path: &Ident<'a>) -> Ident<'a> {
        if let Some(alias) = self.modules.get(module_path) {
            return *alias;
        }

        // Assign new alias. `next_index` is consumed even when the name needs
        // a uniquifying suffix, matching ImportManager's `nextUniqueIndex++`.
        let alias = self.unique_name(&format!("i{}", self.next_index));
        self.next_index += 1;
        self.modules.insert(*module_path, alias);
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
    ///
    /// Modules whose import the file already declares (registered through
    /// [`NamespaceRegistry::with_file_scope`]) are skipped — the existing
    /// import is reused.
    pub fn generate_import_statements(&self) -> String {
        let modules = self.get_all_modules();
        let mut result = String::new();

        for (module_path, alias) in modules {
            if self.reused_modules.contains(module_path) {
                continue;
            }
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
        for name in &other.used_names {
            self.used_names.insert(name.clone());
        }
        for module in &other.reused_modules {
            self.reused_modules.insert(*module);
        }
        for (module_path, alias) in &other.modules {
            if !self.modules.contains_key(module_path) {
                self.modules.insert(*module_path, *alias);
                // Update next_index if the merged alias index is higher
                if let Some(idx_str) = alias.strip_prefix('i')
                    && let Ok(idx) = idx_str.parse::<usize>()
                    && idx >= self.next_index
                {
                    self.next_index = idx + 1;
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

    #[test]
    fn test_core_alias_uniquified_when_i0_taken() {
        let allocator = Allocator::default();
        let mut used = FxHashSet::default();
        used.insert("i0".to_string());
        let registry = NamespaceRegistry::with_file_scope(&allocator, None, Some(used));

        assert_eq!(registry.angular_core_ns().as_str(), "i0_1");
        assert_eq!(
            registry.generate_import_statements(),
            "import * as i0_1 from '@angular/core';\n"
        );
    }

    #[test]
    fn test_core_alias_reused_import_emits_no_import() {
        let allocator = Allocator::default();
        let mut registry =
            NamespaceRegistry::with_file_scope(&allocator, Some(Ident::from("core")), None);

        assert_eq!(registry.angular_core_ns().as_str(), "core");
        assert_eq!(registry.generate_import_statements(), "");

        // Other modules still get imports; the reused one stays skipped.
        registry.get_or_assign(&Ident::from("@angular/router"));
        assert_eq!(
            registry.generate_import_statements(),
            "import * as i1 from '@angular/router';\n"
        );
    }

    #[test]
    fn test_other_alias_uniquified_against_file_identifiers() {
        let allocator = Allocator::default();
        let mut used = FxHashSet::default();
        used.insert("i1".to_string());
        let mut registry = NamespaceRegistry::with_file_scope(&allocator, None, Some(used));

        // i1 is taken by the file, so the first non-core module gets i1_1;
        // the index is still consumed (upstream nextUniqueIndex++).
        let router = registry.get_or_assign(&Ident::from("@angular/router"));
        assert_eq!(router.as_str(), "i1_1");
        let forms = registry.get_or_assign(&Ident::from("@angular/forms"));
        assert_eq!(forms.as_str(), "i2");
    }
}
