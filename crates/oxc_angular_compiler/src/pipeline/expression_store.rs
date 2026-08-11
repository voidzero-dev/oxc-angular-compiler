//! Expression store for managing expressions by reference.
//!
//! This module implements the Reference + Index pattern for expression handling,
//! allowing expressions to be stored once and referenced by ID throughout the
//! compilation pipeline.
//!
//! This avoids the need for deep cloning expressions when they are used in
//! multiple places in the IR.

use oxc_allocator::{Allocator, Vec};

use crate::ast::expression::AngularExpression;

/// A unique identifier for an expression in the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExpressionId(pub u32);

impl ExpressionId {
    /// Creates a new expression ID.
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns the raw ID value.
    pub fn index(&self) -> usize {
        self.0 as usize
    }
}

/// A store for expressions that allows referencing by ID.
///
/// Expressions are stored in an arena-allocated vector and can be referenced
/// by their `ExpressionId`. This allows the same expression to be used in
/// multiple places without cloning.
pub struct ExpressionStore<'a> {
    /// The stored expressions.
    expressions: Vec<'a, AngularExpression<'a>>,
}

impl<'a> ExpressionStore<'a> {
    /// Creates a new expression store.
    pub fn new(allocator: &'a Allocator) -> Self {
        Self { expressions: Vec::new_in(&allocator) }
    }

    /// Stores an expression and returns its ID.
    pub fn store(&mut self, expr: AngularExpression<'a>) -> ExpressionId {
        let id = ExpressionId::new(self.expressions.len() as u32);
        self.expressions.push(expr);
        id
    }

    /// Retrieves an expression by its ID.
    ///
    /// # Panics
    ///
    /// Panics if the ID is out of bounds.
    pub fn get(&self, id: ExpressionId) -> &AngularExpression<'a> {
        &self.expressions[id.index()]
    }

    /// Retrieves a mutable reference to an expression by its ID.
    ///
    /// # Panics
    ///
    /// Panics if the ID is out of bounds.
    pub fn get_mut(&mut self, id: ExpressionId) -> &mut AngularExpression<'a> {
        &mut self.expressions[id.index()]
    }

    /// Returns the number of stored expressions.
    pub fn len(&self) -> usize {
        self.expressions.len()
    }

    /// Returns true if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.expressions.is_empty()
    }

    /// Iterates over all stored expressions with their IDs.
    pub fn iter(&self) -> impl Iterator<Item = (ExpressionId, &AngularExpression<'a>)> {
        self.expressions.iter().enumerate().map(|(i, expr)| (ExpressionId::new(i as u32), expr))
    }

    /// Iterates mutably over all stored expressions with their IDs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (ExpressionId, &mut AngularExpression<'a>)> {
        self.expressions.iter_mut().enumerate().map(|(i, expr)| (ExpressionId::new(i as u32), expr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expression_store() {
        use crate::ast::expression::{AbsoluteSourceSpan, EmptyExpr, ParseSpan};

        let allocator = Allocator::default();
        let mut store = ExpressionStore::new(&allocator);

        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        // Store an expression
        let expr = AngularExpression::Empty(oxc_allocator::Box::new_in(
            EmptyExpr { span: ParseSpan::new(0, 0), source_span: AbsoluteSourceSpan::new(0, 0) },
            &&allocator,
        ));
        let id = store.store(expr);

        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
        assert_eq!(id.index(), 0);

        // Retrieve the expression
        let retrieved = store.get(id);
        assert!(matches!(retrieved, AngularExpression::Empty(_)));
    }
}
