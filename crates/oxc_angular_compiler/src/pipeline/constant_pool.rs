//! Constant pool for deduplicating values.
//!
//! The constant pool stores unique values that can be shared across
//! the compiled output, reducing code size by avoiding duplicate literals.
//!
//! Ported from Angular's `constant_pool.ts`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;
use rustc_hash::FxHashMap;

use crate::ir::expression::IrExpression;
use crate::output::ast::{
    DeclareFunctionStmt, DeclareVarStmt, FunctionExpr, OutputExpression, OutputStatement,
    ReadVarExpr, StmtModifier,
};

/// A pool for deduplicating constant values.
///
/// Constants are keyed by a string representation and deduplicated
/// to reduce the size of generated code.
pub struct ConstantPool<'a> {
    /// Allocator for this pool.
    allocator: &'a Allocator,
    /// Pooled string literals, keyed by value.
    strings: FxHashMap<String, PooledValue>,
    /// Pooled arrays, keyed by a hash of their contents.
    arrays: FxHashMap<u64, PooledValue>,
    /// All pooled values in order.
    values: Vec<'a, PooledConstant<'a>>,
    /// Counter for generating unique names (_c0, _c1, etc.).
    next_name_index: u32,
    /// Global offset for unique_name calls.
    /// This is used to ensure unique names don't conflict when compiling
    /// multiple components in the same file.
    unique_name_offset: u32,
    /// Claimed names for unique_name deduplication within a single component.
    /// Maps base name to usage count.
    claimed_names: FxHashMap<String, u32>,
    /// Statements to emit (e.g., child view functions).
    pub statements: Vec<'a, crate::output::ast::OutputStatement<'a>>,
}

impl<'a> ConstantPool<'a> {
    /// Creates a new constant pool.
    pub fn new(allocator: &'a Allocator) -> Self {
        Self {
            allocator,
            strings: FxHashMap::default(),
            arrays: FxHashMap::default(),
            values: Vec::new_in(allocator),
            next_name_index: 0,
            unique_name_offset: 0,
            claimed_names: FxHashMap::default(),
            statements: Vec::new_in(allocator),
        }
    }

    /// Creates a new constant pool with specific starting indices.
    ///
    /// This is used when compiling multiple components in the same file
    /// to ensure constant names don't conflict. Each component continues
    /// from where the previous component's pool left off.
    ///
    /// For example, if component 1 uses _c0, _c1, _c2, then component 2
    /// should start with _c3 to avoid duplicate const declarations.
    ///
    /// # Arguments
    ///
    /// * `starting_index` - Starting index for constant names (_c0, _c1, etc.)
    ///   AND unique names (_forTrack0, _forTrack1, etc.). Both use the same
    ///   starting index for simplicity.
    pub fn with_starting_index(allocator: &'a Allocator, starting_index: u32) -> Self {
        Self {
            allocator,
            strings: FxHashMap::default(),
            arrays: FxHashMap::default(),
            values: Vec::new_in(allocator),
            next_name_index: starting_index,
            unique_name_offset: starting_index,
            claimed_names: FxHashMap::default(),
            statements: Vec::new_in(allocator),
        }
    }

    /// Returns the next name index that would be used.
    ///
    /// This returns the maximum of next_name_index and the total unique names
    /// generated (offset + claimed count), ensuring proper state transfer when
    /// compiling multiple components in the same file.
    pub fn next_name_index(&self) -> u32 {
        // Calculate the total unique names generated: offset + sum of all claimed counts
        let total_unique_names: u32 =
            self.unique_name_offset + self.claimed_names.values().sum::<u32>();
        self.next_name_index.max(total_unique_names)
    }

    /// Returns the allocator.
    pub fn allocator(&self) -> &'a Allocator {
        self.allocator
    }

    /// Pools a string literal and returns its index.
    pub fn pool_string(&mut self, value: &str) -> u32 {
        if let Some(pooled) = self.strings.get(value) {
            return pooled.index;
        }

        let index = self.values.len() as u32;
        let name = self.generate_name("_c");
        let atom = Ident::from(self.allocator.alloc_str(value));

        self.values.push(PooledConstant {
            name: name.clone(),
            value: PooledValue { index },
            kind: PooledConstantKind::String(atom),
        });

        self.strings.insert(value.to_string(), PooledValue { index });
        index
    }

    /// Pools an array literal and returns its index.
    pub fn pool_array(&mut self, key: u64) -> u32 {
        if let Some(pooled) = self.arrays.get(&key) {
            return pooled.index;
        }

        let index = self.values.len() as u32;
        let name = self.generate_name("_c");

        self.values.push(PooledConstant {
            name,
            value: PooledValue { index },
            kind: PooledConstantKind::ArrayPlaceholder,
        });

        self.arrays.insert(key, PooledValue { index });
        index
    }

    /// Updates an array constant with its actual value.
    pub fn set_array_value(&mut self, index: u32, elements: Vec<'a, PooledConstantKind<'a>>) {
        if let Some(constant) = self.values.get_mut(index as usize) {
            constant.kind = PooledConstantKind::Array(elements);
        }
    }

    /// Returns all pooled constants (immutable).
    pub fn constants(&self) -> &[PooledConstant<'a>] {
        self.values.as_slice()
    }

    /// Returns all pooled constants (mutable).
    /// Used during emit to take ownership of expressions that can't be cloned.
    pub fn constants_mut(&mut self) -> &mut [PooledConstant<'a>] {
        self.values.as_mut_slice()
    }

    /// Returns the number of pooled constants.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Generates a unique name for a pooled constant.
    fn generate_name(&mut self, prefix: &str) -> Ident<'a> {
        let name = format!("{}{}", prefix, self.next_name_index);
        self.next_name_index += 1;
        Ident::from(self.allocator.alloc_str(&name))
    }

    /// Gets the constant at the given index.
    pub fn get(&self, index: u32) -> Option<&PooledConstant<'a>> {
        self.values.get(index as usize)
    }

    /// Pools a regular expression literal and returns its index.
    ///
    /// Only non-global regexes should be pooled (caller responsibility to check).
    pub fn pool_regex(&mut self, body: &str, flags: Option<&str>) -> u32 {
        // Create a key for deduplication
        let key = format!("/{}/{}", body, flags.unwrap_or(""));

        // Check if already pooled (using strings map for simplicity)
        if let Some(pooled) = self.strings.get(&key) {
            return pooled.index;
        }

        let index = self.values.len() as u32;
        let name = self.generate_name("_c");
        let body_atom = Ident::from(self.allocator.alloc_str(body));
        let flags_atom = flags.map(|f| Ident::from(self.allocator.alloc_str(f)));

        self.values.push(PooledConstant {
            name: name.clone(),
            value: PooledValue { index },
            kind: PooledConstantKind::RegularExpression(RegexConstant {
                body: body_atom,
                flags: flags_atom,
            }),
        });

        self.strings.insert(key, PooledValue { index });
        index
    }

    /// Pools a pure function and returns its index and generated name.
    ///
    /// Pure functions are deduplicated by their key (a hash of the body expression).
    /// The caller provides the number of parameters, a string key for deduplication,
    /// and the actual body expression to store and emit.
    ///
    /// # Arguments
    ///
    /// * `num_args` - Number of parameters for the pure function
    /// * `body_key` - String key used for deduplication (typically a debug representation)
    /// * `body_expr` - The actual IR expression to store as the function body
    ///
    /// # Returns
    ///
    /// A tuple of (index, function_name) where the function name can be used to
    /// reference this pooled function.
    pub fn pool_pure_function(
        &mut self,
        num_args: u32,
        body_key: &str,
        body_expr: IrExpression<'a>,
    ) -> (u32, Ident<'a>) {
        // Create a key for deduplication based on arg count and body
        let key = format!("pf:{}:{}", num_args, body_key);

        // Check if already pooled
        if let Some(pooled) = self.strings.get(&key) {
            // Return existing index and name
            let constant = &self.values[pooled.index as usize];
            return (pooled.index, constant.name.clone());
        }

        let index = self.values.len() as u32;
        let name = self.generate_name("_c");

        // Generate parameter names: a0, a1, a2, ...
        let mut params = Vec::with_capacity_in(num_args as usize, self.allocator);
        for i in 0..num_args {
            params.push(Ident::from(self.allocator.alloc_str(&format!("a{}", i))));
        }

        // Store the actual body expression
        self.values.push(PooledConstant {
            name: name.clone(),
            value: PooledValue { index },
            kind: PooledConstantKind::PureFunction(Box::new_in(
                PureFunctionDef { params, body: body_expr },
                self.allocator,
            )),
        });

        self.strings.insert(key, PooledValue { index });
        (index, name)
    }

    /// Gets or creates a shared function reference.
    ///
    /// This is used for defer block dependency resolver functions and track functions.
    /// The function is added to the constant pool's statements and a reference expression
    /// is returned. If an equivalent function already exists, a reference to the existing
    /// function is returned instead.
    ///
    /// Angular TypeScript stores ALL shared functions (both arrow and regular) in the
    /// same `statements` array, maintaining insertion order. This is critical for correct
    /// output ordering - track functions that use `this` (regular function declarations)
    /// and track functions that don't (arrow functions as const declarations) must be
    /// emitted in the order they're encountered.
    ///
    /// Ported from Angular's `ConstantPool.getSharedFunctionReference()`.
    ///
    /// # Arguments
    ///
    /// * `fn_expr` - The function expression to share (ArrowFunction or Function)
    /// * `name` - The name to use for the shared function
    /// * `use_unique_name` - Whether to generate a unique name (false for TDB compatibility)
    ///
    /// # Returns
    ///
    /// An `OutputExpression::ReadVar` that references the shared function.
    pub fn get_shared_function_reference(
        &mut self,
        fn_expr: OutputExpression<'a>,
        name: &str,
        use_unique_name: bool,
    ) -> OutputExpression<'a> {
        let is_arrow = matches!(fn_expr, OutputExpression::ArrowFunction(_));

        // Check if an equivalent function already exists in statements
        // Angular TypeScript iterates through statements checking isEquivalent
        for stmt in self.statements.iter() {
            match stmt {
                // Arrow functions are saved as DeclareVarStmt
                OutputStatement::DeclareVar(decl) if is_arrow => {
                    if let Some(value) = &decl.value {
                        if value.is_equivalent(&fn_expr) {
                            // Return a reference to the existing function
                            return OutputExpression::ReadVar(Box::new_in(
                                ReadVarExpr { name: decl.name.clone(), source_span: None },
                                self.allocator,
                            ));
                        }
                    }
                }
                // Function declarations are saved as DeclareFunctionStmt
                OutputStatement::DeclareFunction(decl) if !is_arrow => {
                    // Compare the function expression with the declaration
                    if let OutputExpression::Function(fn_box) = &fn_expr {
                        if fn_decl_is_equivalent(fn_box, decl) {
                            // Return a reference to the existing function
                            return OutputExpression::ReadVar(Box::new_in(
                                ReadVarExpr { name: decl.name.clone(), source_span: None },
                                self.allocator,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }

        // No equivalent function found, create a new entry
        let fn_name = self.unique_name(name, use_unique_name);

        // Add the function to statements (not values) to match Angular TypeScript
        // Arrow functions become DeclareVarStmt, regular functions become DeclareFunctionStmt
        match fn_expr {
            OutputExpression::ArrowFunction(arrow) => {
                // Arrow function: const _forTrack0 = ($index, $item) => expr;
                self.statements.push(OutputStatement::DeclareVar(Box::new_in(
                    DeclareVarStmt {
                        name: fn_name.clone(),
                        value: Some(OutputExpression::ArrowFunction(arrow)),
                        modifiers: StmtModifier::FINAL,
                        leading_comment: None,
                        source_span: None,
                    },
                    self.allocator,
                )));
            }
            OutputExpression::Function(func) => {
                // Function expression: function _forTrack0($index, $item) { return expr; }
                // Convert FunctionExpr to DeclareFunctionStmt
                // Use unbox to take ownership of the inner FunctionExpr
                let inner = func.unbox();
                self.statements.push(OutputStatement::DeclareFunction(Box::new_in(
                    DeclareFunctionStmt {
                        name: fn_name.clone(),
                        params: inner.params,
                        statements: inner.statements,
                        modifiers: StmtModifier::FINAL,
                        source_span: inner.source_span,
                    },
                    self.allocator,
                )));
            }
            _ => {
                // Other expression types are stored as DeclareVarStmt
                self.statements.push(OutputStatement::DeclareVar(Box::new_in(
                    DeclareVarStmt {
                        name: fn_name.clone(),
                        value: Some(fn_expr),
                        modifiers: StmtModifier::FINAL,
                        leading_comment: None,
                        source_span: None,
                    },
                    self.allocator,
                )));
            }
        }

        // Return a ReadVar expression that references this shared function
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: fn_name, source_span: None },
            self.allocator,
        ))
    }

    /// Generates a unique name with the given base name.
    ///
    /// Used by naming phases to ensure pool-wide uniqueness.
    /// Matches Angular's `ConstantPool.uniqueName()`.
    ///
    /// When compiling multiple components in the same file, the global
    /// unique_name_offset ensures unique names across all components for
    /// "generic" names like `_forTrack` that don't include a component name.
    ///
    /// For component-specific names (like `ComponentName_Template`), the offset
    /// is NOT applied because these names are already unique by virtue of
    /// containing the component class name.
    ///
    /// # Arguments
    ///
    /// * `base` - The base name to make unique
    /// * `always_include_suffix` - If `true`, always append a numeric suffix.
    ///   If `false`, only append suffix if the name has been used before.
    pub fn unique_name(&mut self, base: &str, always_include_suffix: bool) -> Ident<'a> {
        // Get the per-base-name count for deduplication within this component
        let count = self.claimed_names.get(base).copied().unwrap_or(0);

        // Only apply the global offset for "generic" names that start with underscore
        // (like `_forTrack`, `_c`). These names could collide across components.
        // Component-specific names (like `ComponentName_Template`) don't need the
        // offset because they already contain the component class name.
        let apply_offset = base.starts_with('_');
        let effective_count = if apply_offset { self.unique_name_offset + count } else { count };

        let name = if effective_count == 0 && !always_include_suffix {
            base.to_string()
        } else {
            format!("{}{}", base, effective_count)
        };

        // Increment the per-base-name counter
        self.claimed_names.insert(base.to_string(), count + 1);

        Ident::from(self.allocator.alloc_str(&name))
    }

    /// Gets or creates a constant literal and returns a reference expression.
    ///
    /// This is used to share constant literals (arrays, primitives) across the output.
    /// If `share` is true, the constant may be deduplicated with existing constants.
    ///
    /// Ported from Angular's `ConstantPool.getConstLiteral()`.
    ///
    /// # Arguments
    ///
    /// * `literal` - The literal expression to pool
    /// * `share` - Whether to attempt deduplication (currently always adds new constant)
    ///
    /// # Returns
    ///
    /// An `OutputExpression::ReadVar` that references the pooled constant.
    pub fn get_const_literal(
        &mut self,
        literal: OutputExpression<'a>,
        _share: bool,
    ) -> OutputExpression<'a> {
        // Generate a key from the expression for deduplication
        let key = expression_to_key(&literal);

        // Check if already pooled
        if let Some(pooled) = self.strings.get(&key) {
            let constant = &self.values[pooled.index as usize];
            return OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: constant.name.clone(), source_span: None },
                self.allocator,
            ));
        }

        // Add new constant
        let index = self.values.len() as u32;
        let name = self.generate_name("_c");

        self.values.push(PooledConstant {
            name: name.clone(),
            value: PooledValue { index },
            kind: PooledConstantKind::Literal(Box::new_in(literal, self.allocator)),
        });

        self.strings.insert(key, PooledValue { index });

        // Return a ReadVar expression referencing the constant
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name, source_span: None },
            self.allocator,
        ))
    }
}

/// Generates a string key from an OutputExpression for deduplication.
fn expression_to_key(expr: &OutputExpression<'_>) -> String {
    use crate::output::ast::*;

    match expr {
        OutputExpression::Literal(lit) => match &lit.value {
            LiteralValue::Null => "null".to_string(),
            LiteralValue::Undefined => "undefined".to_string(),
            LiteralValue::Boolean(b) => format!("bool:{}", b),
            LiteralValue::Number(n) => format!("num:{}", n),
            LiteralValue::String(s) => format!("str:{}", s),
        },
        OutputExpression::LiteralArray(arr) => {
            let entries: std::vec::Vec<String> =
                arr.entries.iter().map(expression_to_key).collect();
            format!("[{}]", entries.join(","))
        }
        OutputExpression::ReadVar(rv) => format!("var:{}", rv.name),
        // For other types, use a unique identifier to prevent false matches
        _ => format!("expr:{:p}", expr),
    }
}

/// A pooled value reference.
#[derive(Debug, Clone, Copy)]
pub struct PooledValue {
    /// Index in the constant pool.
    pub index: u32,
}

/// A constant stored in the pool.
#[derive(Debug)]
pub struct PooledConstant<'a> {
    /// Generated variable name.
    pub name: Ident<'a>,
    /// Value reference.
    pub value: PooledValue,
    /// The constant's kind and value.
    pub kind: PooledConstantKind<'a>,
}

/// The kind of a pooled constant.
#[derive(Debug)]
pub enum PooledConstantKind<'a> {
    /// A string literal.
    String(Ident<'a>),
    /// A number literal.
    Number(f64),
    /// A boolean literal.
    Boolean(bool),
    /// An array literal.
    Array(Vec<'a, PooledConstantKind<'a>>),
    /// A placeholder for an array (content not yet known).
    ArrayPlaceholder,
    /// An object literal.
    Object(Vec<'a, (Ident<'a>, PooledConstantKind<'a>)>),
    /// An external reference.
    External(ExternalReference<'a>),
    /// A pure function wrapper.
    PureFunction(Box<'a, PureFunctionDef<'a>>),
    /// A regular expression literal.
    RegularExpression(RegexConstant<'a>),
    /// A literal expression (from getConstLiteral).
    /// Stores the full OutputExpression for emission.
    Literal(Box<'a, OutputExpression<'a>>),
}

/// A regular expression constant.
#[derive(Debug)]
pub struct RegexConstant<'a> {
    /// The regex pattern body.
    pub body: Ident<'a>,
    /// The regex flags.
    pub flags: Option<Ident<'a>>,
}

/// An external reference (import).
#[derive(Debug)]
pub struct ExternalReference<'a> {
    /// Module to import from.
    pub module: Ident<'a>,
    /// Name to import.
    pub name: Ident<'a>,
}

/// A pure function definition.
///
/// Pure functions are memoized expressions that are extracted and pooled
/// for deduplication. The body expression is the actual IR expression that
/// will be emitted as an arrow function body.
///
/// Ported from Angular's `PureFunctionConstant` in `constant_pool.ts`.
#[derive(Debug)]
pub struct PureFunctionDef<'a> {
    /// Parameter names (a0, a1, a2, ...).
    pub params: Vec<'a, Ident<'a>>,
    /// Body expression - the actual IR expression to emit.
    ///
    /// This expression may contain `PureFunctionParameterExpr` nodes that
    /// should be transformed to variable references (a0, a1, etc.) during emission.
    pub body: IrExpression<'a>,
}

/// Compare a FunctionExpr with a DeclareFunctionStmt for equivalence.
///
/// This is used by `get_shared_function_reference` to check if an equivalent
/// function already exists in the pool's statements. The comparison ignores
/// names (since the FunctionExpr may not have a name yet) and focuses on
/// params and statements.
fn fn_decl_is_equivalent(func: &FunctionExpr<'_>, decl: &DeclareFunctionStmt<'_>) -> bool {
    // Check params match
    if func.params.len() != decl.params.len() {
        return false;
    }
    if !func.params.iter().zip(decl.params.iter()).all(|(x, y)| x.name == y.name) {
        return false;
    }
    // Check statements match
    if func.statements.len() != decl.statements.len() {
        return false;
    }
    // Compare statements using the output AST's statement comparison
    func.statements.iter().zip(decl.statements.iter()).all(|(x, y)| output_stmt_is_equivalent(x, y))
}

/// Compare two output statements for equivalence.
///
/// Simplified version for comparing function bodies. Handles the common
/// cases of return statements and expression statements.
fn output_stmt_is_equivalent(a: &OutputStatement<'_>, b: &OutputStatement<'_>) -> bool {
    use crate::output::ast::OutputStatement as OS;

    match (a, b) {
        (OS::Return(ra), OS::Return(rb)) => ra.value.is_equivalent(&rb.value),
        (OS::Expression(ea), OS::Expression(eb)) => ea.expr.is_equivalent(&eb.expr),
        (OS::DeclareVar(da), OS::DeclareVar(db)) => {
            da.name == db.name
                && match (&da.value, &db.value) {
                    (Some(va), Some(vb)) => va.is_equivalent(vb),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_deduplication() {
        let allocator = Allocator::default();
        let mut pool = ConstantPool::new(&allocator);

        let idx1 = pool.pool_string("hello");
        let idx2 = pool.pool_string("hello");
        let idx3 = pool.pool_string("world");

        assert_eq!(idx1, idx2);
        assert_ne!(idx1, idx3);
        assert_eq!(pool.len(), 2);
    }
}
