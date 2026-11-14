//! Native attachment interface for the shared WGSL fragment implementation.

use naga::{AddressSpace, Block, Expression, GlobalVariable, Span, Statement};

/// Replace only the two designated corresponding-pixel read functions.
pub fn attachment_inputs(module: &mut naga::Module) {
    for (function_name, global_name) in [
        ("read_composite_source", "attachment_source"),
        ("read_composite_backdrop", "attachment_backdrop"),
    ] {
        let (_, function) = module
            .functions
            .iter_mut()
            .find(|(_, f)| f.name.as_deref() == Some(function_name))
            .expect("the engine defines its attachment read interface");
        let global = module.global_variables.append(
            GlobalVariable {
                name: Some(global_name.into()),
                space: AddressSpace::Private,
                binding: None,
                ty: function.result.as_ref().expect("read returns a colour").ty,
                init: None,
                memory_decorations: naga::MemoryDecorations::empty(),
            },
            Span::UNDEFINED,
        );
        function.expressions.clear();
        function.named_expressions.clear();
        function.local_variables.clear();
        let pointer = function
            .expressions
            .append(Expression::GlobalVariable(global), Span::UNDEFINED);
        let value = function
            .expressions
            .append(Expression::Load { pointer }, Span::UNDEFINED);
        function.body = Block::new();
        function.body.push(
            Statement::Emit(naga::Range::new_from_bounds(value, value)),
            Span::UNDEFINED,
        );
        function
            .body
            .push(Statement::Return { value: Some(value) }, Span::UNDEFINED);
    }
}
