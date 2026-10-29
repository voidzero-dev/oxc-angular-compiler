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
    DeclareFunctionStmt, DeclareVarStmt, FunctionExpr, LiteralValue, OutputExpression,
    OutputStatement, ReadVarExpr, StmtModifier, clone_output_statement,
};

/// Strings at least this long (in UTF-16 code units, like JS `length`) are
/// shared once they are used twice; shorter ones always stay inline.
///
/// Ported from Angular's `POOL_INCLUSION_LENGTH_THRESHOLD_FOR_STRINGS`.
const POOL_INCLUSION_LENGTH_THRESHOLD_FOR_STRINGS: usize = 50;

/// What one class's pool hands to the next class's pool in a file.
///
/// ngtsc compiles every class of a file with one `ConstantPool`, so a literal,
/// pure function or shared function pooled by one class is reused by the
/// next, and a name one class claims is not claimed again by another. Oxc
/// gives each class its own pool (with `_c` names chained by the starting
/// index), so the file transform carries these over with
/// [`ConstantPool::take_shared`] and [`ConstantPool::inherit_shared`].
#[derive(Debug, Default, Clone)]
pub struct SharedPool<'a> {
    /// Literals pooled by `get_const_literal`, by key.
    literals: FxHashMap<String, PooledLiteral<'a>>,
    /// Pure functions pooled by `pool_pure_function`, by key.
    shared_constants: FxHashMap<String, Ident<'a>>,
    /// Functions declared by `get_shared_function_reference`.
    shared_functions: std::vec::Vec<&'a OutputStatement<'a>>,
    /// Names claimed by `unique_name` (`_forTrack`, `Cmp_Template`, ...), with
    /// their usage counts.
    claimed_names: FxHashMap<String, u32>,
}

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
    /// Claimed names for unique_name deduplication, including the names of
    /// the earlier classes of the file. Maps base name to usage count.
    claimed_names: FxHashMap<String, u32>,
    /// Literals pooled by `get_const_literal`, by key.
    literals: FxHashMap<String, PooledLiteral<'a>>,
    /// Pure functions pooled by `pool_pure_function`, including the earlier
    /// classes' of the file, by key.
    shared_constants: FxHashMap<String, Ident<'a>>,
    /// The functions declared by `get_shared_function_reference`, including
    /// the earlier classes' of the file, whose statements may have left
    /// `statements` already.
    shared_functions: std::vec::Vec<&'a OutputStatement<'a>>,
    /// Long strings an earlier class of the file used inline once, shared by
    /// this pool. Their declaration and first use belong to that earlier class.
    inherited_fixups: std::vec::Vec<InheritedFixup<'a>>,
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
            values: Vec::new_in(&allocator),
            next_name_index: 0,
            claimed_names: FxHashMap::default(),
            literals: FxHashMap::default(),
            shared_constants: FxHashMap::default(),
            shared_functions: std::vec::Vec::new(),
            inherited_fixups: std::vec::Vec::new(),
            statements: Vec::new_in(&allocator),
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
    /// * `starting_index` - Starting index for constant names (_c0, _c1, etc.).
    ///   Names from `unique_name` (_forTrack0, ...) are counted per base name
    ///   instead, with the earlier classes' counts from
    ///   [`ConstantPool::inherit_shared`].
    pub fn with_starting_index(allocator: &'a Allocator, starting_index: u32) -> Self {
        Self {
            allocator,
            strings: FxHashMap::default(),
            arrays: FxHashMap::default(),
            values: Vec::new_in(&allocator),
            next_name_index: starting_index,
            claimed_names: FxHashMap::default(),
            literals: FxHashMap::default(),
            shared_constants: FxHashMap::default(),
            shared_functions: std::vec::Vec::new(),
            inherited_fixups: std::vec::Vec::new(),
            statements: Vec::new_in(&allocator),
        }
    }

    /// Returns the next `_c` constant index that would be used.
    ///
    /// Angular numbers `_c` constants (`freshName`) apart from the other
    /// names `uniqueName` hands out, so only constants move it.
    pub fn next_name_index(&self) -> u32 {
        self.next_name_index
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
    /// The function name, which can be used to reference this pooled function.
    /// An equal function an earlier class of the file pooled is reused, as
    /// Angular's `getSharedConstant` does; that class declares it.
    pub fn pool_pure_function(
        &mut self,
        num_args: u32,
        body_key: &str,
        body_expr: IrExpression<'a>,
    ) -> Ident<'a> {
        // Create a key for deduplication based on arg count and body
        let key = format!("pf:{}:{}", num_args, body_key);

        // Check if already pooled
        if let Some(name) = self.shared_constants.get(&key) {
            return *name;
        }

        let index = self.values.len() as u32;
        let name = self.generate_name("_c");

        // Generate parameter names: a0, a1, a2, ...
        let mut params = Vec::with_capacity_in(num_args as usize, &self.allocator);
        for i in 0..num_args {
            params.push(Ident::from(self.allocator.alloc_str(&format!("a{}", i))));
        }

        // Store the actual body expression
        self.values.push(PooledConstant {
            name: name.clone(),
            value: PooledValue { index },
            kind: PooledConstantKind::PureFunction(Box::new_in(
                PureFunctionDef { params, body: body_expr },
                &self.allocator,
            )),
        });

        self.shared_constants.insert(key, name);
        name
    }

    /// Gets or creates a shared function reference.
    ///
    /// This is used for defer block dependency resolver functions and track functions.
    /// The function is added to the constant pool's statements and a reference expression
    /// is returned. If an equivalent function already exists, a reference to the existing
    /// function is returned instead, even one an earlier class of the file declared.
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
        // Angular TypeScript iterates through statements checking isEquivalent.
        // The earlier classes' shared functions come first, and this pool's
        // may have left `statements` already.
        for stmt in self.shared_functions.iter().copied().chain(self.statements.iter()) {
            match stmt {
                // Arrow functions are saved as DeclareVarStmt
                OutputStatement::DeclareVar(decl) if is_arrow => {
                    if let Some(value) = &decl.value {
                        if value.is_equivalent(&fn_expr) {
                            // Return a reference to the existing function
                            return OutputExpression::ReadVar(Box::new_in(
                                ReadVarExpr { name: decl.name.clone(), source_span: None },
                                &self.allocator,
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
                                &self.allocator,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }

        // No equivalent function found, create a new entry. Without
        // `use_unique_name` the name is taken as is and not claimed, so it
        // doesn't move the pool's name indices.
        let fn_name = if use_unique_name {
            self.unique_name(name, true)
        } else {
            Ident::from(self.allocator.alloc_str(name))
        };

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
                    &self.allocator,
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
                    &self.allocator,
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
                    &self.allocator,
                )));
            }
        }
        // Keep a copy for the next classes of the file to reuse.
        if let Some(stmt) = self.statements.last() {
            let stmt = self.allocator.alloc(clone_output_statement(stmt, self.allocator));
            self.shared_functions.push(stmt);
        }

        // Return a ReadVar expression that references this shared function
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: fn_name, source_span: None },
            &self.allocator,
        ))
    }

    /// Generates a unique name with the given base name.
    ///
    /// Used by naming phases to ensure pool-wide uniqueness.
    /// Matches Angular's `ConstantPool.uniqueName()`: each base name has its
    /// own count, and the counts of the earlier classes of the file come in
    /// through [`ConstantPool::inherit_shared`].
    ///
    /// # Arguments
    ///
    /// * `base` - The base name to make unique
    /// * `always_include_suffix` - If `true`, always append a numeric suffix.
    ///   If `false`, only append suffix if the name has been used before.
    pub fn unique_name(&mut self, base: &str, always_include_suffix: bool) -> Ident<'a> {
        // Get the per-base-name count for deduplication across the file
        let count = self.claimed_names.get(base).copied().unwrap_or(0);

        let name = if count == 0 && !always_include_suffix {
            base.to_string()
        } else {
            format!("{base}{count}")
        };

        // Increment the per-base-name counter
        self.claimed_names.insert(base.to_string(), count + 1);

        Ident::from(self.allocator.alloc_str(&name))
    }

    /// Gets or creates a constant literal and returns the expression to use.
    ///
    /// A short string literal is returned as is. A long string (see
    /// `POOL_INCLUSION_LENGTH_THRESHOLD_FOR_STRINGS`) stays inline on its
    /// first use and becomes a shared constant on its second one, unless
    /// `force_shared` asks for the constant right away; any other literal
    /// should be passed with `force_shared`. The inline first use is not
    /// patched here: see [`ConstantPool::shared_literal`] and
    /// [`ConstantPool::take_inherited_fixups`].
    ///
    /// Ported from Angular's `ConstantPool.getConstLiteral()`.
    pub fn get_const_literal(
        &mut self,
        literal: OutputExpression<'a>,
        force_shared: bool,
    ) -> OutputExpression<'a> {
        if matches!(literal, OutputExpression::Literal(_)) && !is_long_string_literal(&literal) {
            return literal;
        }

        let key = expression_to_key(&literal);
        let pooled = self.literals.get(&key).copied();
        if let Some(PooledLiteral { name: Some(name), .. }) = pooled {
            return self.read_var(name);
        }
        if pooled.is_none() && !force_shared {
            // First use of a long string: it stays inline until a second one.
            self.literals.insert(key, PooledLiteral { name: None, inherited: false });
            return literal;
        }

        let name = self.generate_name("_c");
        if pooled.is_some_and(|pooled| pooled.inherited) {
            // The inline first use is in an earlier class, so the constant
            // has to be declared before that class.
            self.inherited_fixups.push(InheritedFixup { name, value: literal });
        } else {
            let index = self.values.len() as u32;
            self.values.push(PooledConstant {
                name,
                value: PooledValue { index },
                kind: PooledConstantKind::Literal(Box::new_in(literal, &self.allocator)),
            });
        }
        self.literals.insert(key, PooledLiteral { name: Some(name), inherited: false });
        self.read_var(name)
    }

    /// Returns the shared constant for `expr` if it is the inline first use of
    /// a long string that a later use has shared since.
    ///
    /// Angular returns a `FixupExpression` for every pooled literal, which the
    /// second use points at the new constant. Callers that may hold a first use
    /// run it through this once all uses are pooled.
    pub fn shared_literal(&self, expr: &OutputExpression<'a>) -> Option<OutputExpression<'a>> {
        if !is_long_string_literal(expr) {
            return None;
        }
        let name = self.literals.get(&expression_to_key(expr))?.name?;
        Some(self.read_var(name))
    }

    /// Takes the literals, functions and names this pool and the ones before
    /// it pooled and claimed, to hand them to the next class's pool.
    pub fn take_shared(&mut self) -> SharedPool<'a> {
        SharedPool {
            literals: std::mem::take(&mut self.literals),
            shared_constants: std::mem::take(&mut self.shared_constants),
            shared_functions: std::mem::take(&mut self.shared_functions),
            claimed_names: self.claimed_names.clone(),
        }
    }

    /// Reuses the literals and functions pooled, and avoids the names claimed,
    /// by the earlier classes of the file.
    pub fn inherit_shared(&mut self, shared: SharedPool<'a>) {
        let SharedPool { mut literals, shared_constants, shared_functions, claimed_names } = shared;
        for pooled in literals.values_mut() {
            pooled.inherited = true;
        }
        self.literals = literals;
        self.shared_constants = shared_constants;
        self.shared_functions = shared_functions;
        self.claimed_names.extend(claimed_names);
    }

    /// Takes the long strings an earlier class used inline once and this pool
    /// shared. The caller declares each before that class and points its
    /// first use at the constant.
    pub fn take_inherited_fixups(&mut self) -> std::vec::Vec<InheritedFixup<'a>> {
        std::mem::take(&mut self.inherited_fixups)
    }

    /// Moves the next constant name past `index`, the next name of a pool
    /// compiled after this one (ngtsc shares one pool between them).
    pub fn continue_from(&mut self, index: u32) {
        self.next_name_index = self.next_name_index.max(index);
    }

    fn read_var(&self, name: Ident<'a>) -> OutputExpression<'a> {
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name, source_span: None },
            &self.allocator,
        ))
    }
}

/// Whether `expr` is a string literal long enough to share.
///
/// Ported from Angular's `isLongStringLiteral`.
pub fn is_long_string_literal(expr: &OutputExpression<'_>) -> bool {
    matches!(
        expr,
        OutputExpression::Literal(lit) if matches!(
            &lit.value,
            LiteralValue::String(value)
                if value.encode_utf16().count() >= POOL_INCLUSION_LENGTH_THRESHOLD_FOR_STRINGS
        )
    )
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
            // Quoted and escaped, so no string can read as several entries.
            LiteralValue::String(s) => format!("str:{s:?}"),
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

/// A literal pooled by [`ConstantPool::get_const_literal`].
#[derive(Debug, Clone, Copy)]
pub struct PooledLiteral<'a> {
    /// The shared constant's name, or `None` while the literal is a long
    /// string used once, inline.
    pub name: Option<Ident<'a>>,
    /// Whether an earlier class of the file pooled the literal.
    pub inherited: bool,
}

/// A long string that an earlier class of the file used inline once and a
/// later class shared, from [`ConstantPool::take_inherited_fixups`].
#[derive(Debug)]
pub struct InheritedFixup<'a> {
    /// The shared constant's name.
    pub name: Ident<'a>,
    /// The literal, inline in the earlier class.
    pub value: OutputExpression<'a>,
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
    use crate::output::ast::{ArrowFunctionBody, ArrowFunctionExpr, LiteralExpr, LiteralValue};

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

    fn string<'a>(allocator: &'a Allocator, value: &str) -> OutputExpression<'a> {
        OutputExpression::Literal(Box::new_in(
            crate::output::ast::LiteralExpr {
                value: LiteralValue::String(Ident::from(allocator.alloc_str(value))),
                source_span: None,
            },
            &allocator,
        ))
    }

    fn read_var_name<'a>(expr: &OutputExpression<'a>) -> Option<&'a str> {
        match expr {
            OutputExpression::ReadVar(read) => Some(read.name.as_str()),
            _ => None,
        }
    }

    #[test]
    fn test_long_string_is_shared_from_its_second_use() {
        let allocator = Allocator::default();
        let mut pool = ConstantPool::new(&allocator);
        let long = "x".repeat(POOL_INCLUSION_LENGTH_THRESHOLD_FOR_STRINGS);

        let first = pool.get_const_literal(string(&allocator, &long), false);
        assert!(matches!(first, OutputExpression::Literal(_)));
        assert!(pool.shared_literal(&first).is_none());
        assert!(pool.is_empty());

        let second = pool.get_const_literal(string(&allocator, &long), false);
        assert_eq!(read_var_name(&second), Some("_c0"));
        assert_eq!(pool.shared_literal(&first).as_ref().and_then(read_var_name), Some("_c0"));
        let third = pool.get_const_literal(string(&allocator, &long), false);
        assert_eq!(read_var_name(&third), Some("_c0"));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_short_string_stays_inline() {
        let allocator = Allocator::default();
        let mut pool = ConstantPool::new(&allocator);
        // 49 UTF-16 code units, though more bytes in UTF-8.
        let short = "é".repeat(POOL_INCLUSION_LENGTH_THRESHOLD_FOR_STRINGS - 1);

        for _ in 0..2 {
            let expr = pool.get_const_literal(string(&allocator, &short), true);
            assert!(matches!(expr, OutputExpression::Literal(_)));
        }
        assert!(pool.is_empty());
    }

    #[test]
    fn test_long_string_first_used_by_an_earlier_pool_is_a_fixup() {
        let allocator = Allocator::default();
        let long = "x".repeat(POOL_INCLUSION_LENGTH_THRESHOLD_FOR_STRINGS);
        let mut first_pool = ConstantPool::new(&allocator);
        first_pool.get_const_literal(string(&allocator, &long), false);

        let mut pool = ConstantPool::with_starting_index(&allocator, 3);
        pool.inherit_shared(first_pool.take_shared());
        let second = pool.get_const_literal(string(&allocator, &long), false);
        assert_eq!(read_var_name(&second), Some("_c3"));
        // Declared before the earlier class, not with this pool's constants.
        assert!(pool.is_empty());
        let fixups = pool.take_inherited_fixups();
        assert_eq!(fixups.len(), 1);
        assert_eq!(fixups[0].name.as_str(), "_c3");
        assert!(fixups[0].value.is_equivalent(&string(&allocator, &long)));
    }

    #[test]
    fn test_unique_names_count_across_pools_apart_from_constants() {
        let allocator = Allocator::default();
        let mut first_pool = ConstantPool::new(&allocator);
        first_pool.pool_string("a");
        first_pool.pool_string("b");
        assert_eq!(first_pool.unique_name("_forTrack", true).as_str(), "_forTrack0");

        let mut pool = ConstantPool::with_starting_index(&allocator, first_pool.next_name_index());
        pool.inherit_shared(first_pool.take_shared());
        assert_eq!(pool.unique_name("_forTrack", true).as_str(), "_forTrack1");
        assert_eq!(pool.unique_name("arrowFn", true).as_str(), "arrowFn0");
        assert_eq!(pool.next_name_index(), 2);
    }

    #[test]
    fn test_shared_function_without_unique_name_takes_no_index() {
        let allocator = &Allocator::default();
        let mut pool = ConstantPool::with_starting_index(allocator, 2);
        let arrow = |value: f64| {
            OutputExpression::ArrowFunction(Box::new_in(
                ArrowFunctionExpr {
                    params: Vec::new_in(&allocator),
                    body: ArrowFunctionBody::Expression(Box::new_in(
                        OutputExpression::Literal(Box::new_in(
                            LiteralExpr { value: LiteralValue::Number(value), source_span: None },
                            &allocator,
                        )),
                        &allocator,
                    )),
                    source_span: None,
                },
                &allocator,
            ))
        };

        let first = pool.get_shared_function_reference(arrow(1.0), "Cmp_Defer_1_DepsFn", false);
        let second = pool.get_shared_function_reference(arrow(2.0), "Cmp_Defer_3_DepsFn", false);

        let name = |expr: &OutputExpression<'_>| match expr {
            OutputExpression::ReadVar(read) => read.name.to_string(),
            _ => panic!("expected a variable reference"),
        };
        assert_eq!(name(&first), "Cmp_Defer_1_DepsFn");
        assert_eq!(name(&second), "Cmp_Defer_3_DepsFn");
        assert_eq!(pool.next_name_index(), 2);
    }
}
