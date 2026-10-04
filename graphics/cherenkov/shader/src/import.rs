//! Copying functions, with everything they depend on, from a snippet module
//! into the composed module.
//!
//! A function's own arenas (expressions, local variables) are copied in
//! order, so every handle local to the function keeps its index. Only the
//! handles into module-level arenas (types, constants, global expressions,
//! functions, diagnostic filters) are remapped.

use std::collections::HashMap;

use naga::{
    ArraySize, Block, Expression, Function, Handle, Module, Span, Statement, Type, TypeInner,
    diagnostic_filter::DiagnosticFilterNode,
};

/// Imports items from one source module into a destination module,
/// remembering what it already imported.
pub struct Importer<'s> {
    src: &'s Module,
    prefix: String,
    types: HashMap<Handle<Type>, Handle<Type>>,
    constants: HashMap<Handle<naga::Constant>, Handle<naga::Constant>>,
    global_expressions: HashMap<Handle<Expression>, Handle<Expression>>,
    functions: HashMap<Handle<Function>, Handle<Function>>,
    diagnostics: HashMap<Handle<DiagnosticFilterNode>, Handle<DiagnosticFilterNode>>,
}

impl<'s> Importer<'s> {
    /// An importer from `src`. Imported functions are renamed
    /// `{prefix}_{name}`; an empty prefix keeps the names as they are.
    pub(crate) fn new(src: &'s Module, prefix: &str) -> Self {
        Self {
            src,
            prefix: if prefix.is_empty() {
                String::new()
            } else {
                identifier(prefix)
            },
            types: HashMap::new(),
            constants: HashMap::new(),
            global_expressions: HashMap::new(),
            functions: HashMap::new(),
            diagnostics: HashMap::new(),
        }
    }

    /// Maps `source` onto an existing `dst` type instead of copying it. Used
    /// for declarations the composer owns canonically, like the working-space
    /// block, whose snippet-side name is free.
    pub(crate) fn alias_type(&mut self, source: Handle<Type>, mapped: Handle<Type>) {
        self.types.insert(source, mapped);
    }

    /// Maps `source` onto an existing `dst` function instead of copying it.
    /// Used for a library function whose canonical copy the composer
    /// already imported — every stage's copy of it is that one function.
    pub(crate) fn alias_function(&mut self, source: Handle<Function>, mapped: Handle<Function>) {
        self.functions.insert(source, mapped);
    }

    /// The source-to-destination function handles imported so far —
    /// every callee a function reached counts as imported too.
    pub(crate) fn imported_functions(
        &self,
    ) -> impl Iterator<Item = (Handle<Function>, Handle<Function>)> + '_ {
        self.functions
            .iter()
            .map(|(&source, &mapped)| (source, mapped))
    }

    pub(crate) fn ty(&mut self, dst: &mut Module, handle: Handle<Type>) -> Handle<Type> {
        if let Some(&mapped) = self.types.get(&handle) {
            return mapped;
        }
        let src = self.src;
        let source = &src.types[handle];
        let inner = match source.inner {
            TypeInner::Pointer { base, space } => TypeInner::Pointer {
                base: self.ty(dst, base),
                space,
            },
            TypeInner::Array { base, size, stride } => TypeInner::Array {
                base: self.ty(dst, base),
                size: array_size(size),
                stride,
            },
            TypeInner::BindingArray { base, size } => TypeInner::BindingArray {
                base: self.ty(dst, base),
                size: array_size(size),
            },
            TypeInner::Struct { ref members, span } => TypeInner::Struct {
                members: members
                    .iter()
                    .map(|member| naga::StructMember {
                        ty: self.ty(dst, member.ty),
                        ..member.clone()
                    })
                    .collect(),
                span,
            },
            ref other => other.clone(),
        };
        let mapped = dst.types.insert(
            Type {
                name: source.name.clone(),
                inner,
            },
            src.types.get_span(handle),
        );
        for (predeclared, &original) in &src.special_types.predeclared_types {
            if original == handle {
                dst.special_types
                    .predeclared_types
                    .insert(predeclared.clone(), mapped);
            }
        }
        self.types.insert(handle, mapped);
        mapped
    }

    fn constant(
        &mut self,
        dst: &mut Module,
        handle: Handle<naga::Constant>,
    ) -> Handle<naga::Constant> {
        if let Some(&mapped) = self.constants.get(&handle) {
            return mapped;
        }
        let src = self.src;
        let source = &src.constants[handle];
        let constant = naga::Constant {
            name: source.name.as_deref().map(|name| self.renamed(name)),
            ty: self.ty(dst, source.ty),
            init: self.global_expression(dst, source.init),
        };
        let mapped = dst
            .constants
            .append(constant, src.constants.get_span(handle));
        self.constants.insert(handle, mapped);
        mapped
    }

    fn global_expression(
        &mut self,
        dst: &mut Module,
        handle: Handle<Expression>,
    ) -> Handle<Expression> {
        if let Some(&mapped) = self.global_expressions.get(&handle) {
            return mapped;
        }
        let src = self.src;
        let expression = match src.global_expressions[handle] {
            Expression::Constant(constant) => Expression::Constant(self.constant(dst, constant)),
            Expression::ZeroValue(ty) => Expression::ZeroValue(self.ty(dst, ty)),
            Expression::Compose { ty, ref components } => Expression::Compose {
                ty: self.ty(dst, ty),
                components: components
                    .iter()
                    .map(|&component| self.global_expression(dst, component))
                    .collect(),
            },
            ref other => crate::rewrite::map_expression(other, |operand| {
                self.global_expression(dst, operand)
            }),
        };
        let mapped = dst
            .global_expressions
            .append(expression, src.global_expressions.get_span(handle));
        self.global_expressions.insert(handle, mapped);
        mapped
    }

    fn diagnostic(
        &mut self,
        dst: &mut Module,
        handle: Handle<DiagnosticFilterNode>,
    ) -> Handle<DiagnosticFilterNode> {
        if let Some(&mapped) = self.diagnostics.get(&handle) {
            return mapped;
        }
        let src = self.src;
        let source = &src.diagnostic_filters[handle];
        let node = DiagnosticFilterNode {
            inner: source.inner.clone(),
            parent: source.parent.map(|parent| self.diagnostic(dst, parent)),
        };
        let mapped = dst
            .diagnostic_filters
            .append(node, src.diagnostic_filters.get_span(handle));
        self.diagnostics.insert(handle, mapped);
        mapped
    }

    /// Imports a function and, first, every function it calls.
    pub(crate) fn function(
        &mut self,
        dst: &mut Module,
        handle: Handle<Function>,
    ) -> Handle<Function> {
        if let Some(&mapped) = self.functions.get(&handle) {
            return mapped;
        }
        let src = self.src;
        let mut function = src.functions[handle].clone();
        function.name = function.name.as_deref().map(|name| self.renamed(name));
        for argument in &mut function.arguments {
            argument.ty = self.ty(dst, argument.ty);
        }
        if let Some(result) = function.result.as_mut() {
            result.ty = self.ty(dst, result.ty);
        }
        for (_, local) in function.local_variables.iter_mut() {
            local.ty = self.ty(dst, local.ty);
        }
        for (_, expression) in function.expressions.iter_mut() {
            self.remap_external(dst, expression);
        }
        self.remap_calls(dst, &mut function.body);
        function.diagnostic_filter_leaf = function
            .diagnostic_filter_leaf
            .map(|leaf| self.diagnostic(dst, leaf));
        let mapped = dst
            .functions
            .append(function, src.functions.get_span(handle));
        self.functions.insert(handle, mapped);
        mapped
    }

    /// Remaps the module-level handles inside one function-local expression.
    fn remap_external(&mut self, dst: &mut Module, expression: &mut Expression) {
        match expression {
            Expression::Constant(constant) => *constant = self.constant(dst, *constant),
            Expression::ZeroValue(ty)
            | Expression::Compose { ty, .. }
            | Expression::AtomicResult { ty, .. }
            | Expression::WorkGroupUniformLoadResult { ty }
            | Expression::SubgroupOperationResult { ty } => *ty = self.ty(dst, *ty),
            Expression::CallResult(function) => *function = self.function(dst, *function),
            // Parsed snippets are checked to use no overrides and no global
            // variables; every other expression refers only to function-local
            // handles or plain data.
            _ => {}
        }
    }

    fn remap_calls(&mut self, dst: &mut Module, block: &mut Block) {
        for statement in block.iter_mut() {
            match statement {
                Statement::Call { function, .. } => *function = self.function(dst, *function),
                Statement::Block(inner) => self.remap_calls(dst, inner),
                Statement::If { accept, reject, .. } => {
                    self.remap_calls(dst, accept);
                    self.remap_calls(dst, reject);
                }
                Statement::Switch { cases, .. } => {
                    for case in cases {
                        self.remap_calls(dst, &mut case.body);
                    }
                }
                Statement::Loop {
                    body, continuing, ..
                } => {
                    self.remap_calls(dst, body);
                    self.remap_calls(dst, continuing);
                }
                _ => {}
            }
        }
    }

    fn renamed(&self, name: &str) -> String {
        if self.prefix.is_empty() {
            return name.to_owned();
        }
        format!("{}_{name}", self.prefix)
    }
}

fn array_size(size: ArraySize) -> ArraySize {
    match size {
        ArraySize::Constant(length) => ArraySize::Constant(length),
        ArraySize::Dynamic => ArraySize::Dynamic,
        // Parsed snippets are checked to declare no overrides, so no array
        // length can be pending on one.
        ArraySize::Pending(_) => unreachable!("snippets declare no overrides"),
    }
}

/// Turns a snippet name into a WGSL identifier fragment.
pub fn identifier(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 's');
    }
    out
}

/// The span assigned to generated items.
pub const GENERATED: Span = Span::UNDEFINED;
