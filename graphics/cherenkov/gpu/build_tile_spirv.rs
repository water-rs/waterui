//! SPIR-V input attachment reads for the shared WGSL fragment bodies.
//!
//! naga has no representation for an input attachment — no `SubpassData`
//! image, no `InputAttachmentIndex` decoration — so the Vulkan tile
//! interface cannot be emitted the way `attachment.metal` is written. It
//! is produced instead by rewriting naga's own emission, which keeps the
//! shaded body bit-identical to the portable pipelines' body by
//! construction: `build_tile.rs` has already reduced each
//! corresponding-pixel read to a load of a private global, and this pass
//! replaces exactly those loads with `OpImageRead` of a `SubpassData`
//! image.
//!
//! The two reads become input attachment indices 0 (the draw's own slot,
//! its backdrop) and `source_index`. A draw that sources its own slot
//! passes 0 for both, because an attachment cannot be mapped to two
//! input attachment indices at once
//! (`VUID-VkRenderingInputAttachmentIndexInfo-pColorAttachmentInputIndices-09522`);
//! every other pair reads the source at index 1.

use rspirv::binary::Assemble;
use rspirv::dr::{Instruction, Module, Operand};
use rspirv::spirv::{Capability, Decoration, Dim, ImageFormat, Op, StorageClass, Word};

/// The private globals `build_tile.rs` leaves in place of the reads, in
/// the input attachment index order this pass assigns them.
const READS: [&str; 2] = ["attachment_backdrop", "attachment_source"];

/// Rewrites the required corresponding-pixel reads of `words` into input
/// attachment reads, the source read at `source_index`.
///
/// Panics on anything it does not precisely model: a missing required read global,
/// a read whose type is not the emitted colour type, or a use of a read
/// global other than the load this pass replaces. A tile shader that
/// reaches the device must read the attachments it was planned around, so
/// a drifted emission fails the build rather than lowering to a sampled
/// read of undefined private storage.
#[expect(
    clippy::too_many_lines,
    reason = "one SPIR-V rewriting pass, like `merge_sampled_pair`"
)]
pub fn attachment_reads(
    words: &[u32],
    set: u32,
    bindings: [u32; 2],
    source_index: u32,
    used: [bool; 2],
) -> Vec<u32> {
    let mut module = rspirv::dr::load_words(words).expect("naga emitted parseable SPIR-V");
    let globals: Vec<_> = READS
        .into_iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let global = read_global(&module, name);
            assert_eq!(
                global.is_some(),
                used[index],
                "unexpected attachment usage for {name}"
            );
            global.map(|global| (index, global))
        })
        .collect();
    if globals.is_empty() {
        module.debug_names.clear();
        return module.assemble();
    }
    let colour = pointee(&module, globals[0].1);
    for &(_, global) in &globals {
        assert_eq!(
            colour,
            pointee(&module, global),
            "attachment colour types disagree"
        );
    }

    let mut next = module.header.as_mut().expect("SPIR-V header").bound;
    module.capabilities.push(Instruction::new(
        Op::Capability,
        None,
        None,
        vec![Operand::Capability(Capability::InputAttachment)],
    ));
    let float_ty = component(&module, colour);
    // `Sampled` is 2: an input attachment is read without a sampler.
    let image_ty = intern(
        &mut module,
        &mut next,
        Op::TypeImage,
        None,
        vec![
            Operand::IdRef(float_ty),
            Operand::Dim(Dim::DimSubpassData),
            Operand::LiteralBit32(0),
            Operand::LiteralBit32(0),
            Operand::LiteralBit32(0),
            Operand::LiteralBit32(2),
            Operand::ImageFormat(ImageFormat::Unknown),
        ],
    );
    let pointer_ty = intern(
        &mut module,
        &mut next,
        Op::TypePointer,
        None,
        vec![
            Operand::StorageClass(StorageClass::UniformConstant),
            Operand::IdRef(image_ty),
        ],
    );
    let int_ty = intern(
        &mut module,
        &mut next,
        Op::TypeInt,
        None,
        vec![Operand::LiteralBit32(32), Operand::LiteralBit32(1)],
    );
    let coordinate_ty = intern(
        &mut module,
        &mut next,
        Op::TypeVector,
        None,
        vec![Operand::IdRef(int_ty), Operand::LiteralBit32(2)],
    );
    let zero = intern(
        &mut module,
        &mut next,
        Op::Constant,
        Some(int_ty),
        vec![Operand::LiteralBit32(0)],
    );
    // The read is of the fragment's own pixel; `OpImageRead` of a
    // `SubpassData` image takes the offset from it, which is zero.
    let origin = intern(
        &mut module,
        &mut next,
        Op::ConstantComposite,
        Some(coordinate_ty),
        vec![Operand::IdRef(zero), Operand::IdRef(zero)],
    );
    let images: Vec<_> = globals
        .iter()
        .map(|&(index, global)| {
            let image = next;
            next += 1;
            module.types_global_values.push(Instruction::new(
                Op::Variable,
                Some(pointer_ty),
                Some(image),
                vec![Operand::StorageClass(StorageClass::UniformConstant)],
            ));
            (index, global, image)
        })
        .collect();

    for &(index, _, image) in &images {
        let input_index = if index == 0 { 0 } else { source_index };
        for (decoration, value) in [
            (Decoration::DescriptorSet, set),
            (Decoration::Binding, bindings[index]),
            (Decoration::InputAttachmentIndex, input_index),
        ] {
            module.annotations.push(Instruction::new(
                Op::Decorate,
                None,
                None,
                vec![
                    Operand::IdRef(image),
                    Operand::Decoration(decoration),
                    Operand::LiteralBit32(value),
                ],
            ));
        }
    }

    for (_, global, image) in images {
        let replaced = replace_loads(
            &mut module,
            global,
            (image, image_ty),
            colour,
            origin,
            &mut next,
        );
        assert_eq!(
            replaced, 1,
            "the attachment read interface is one load per read"
        );
    }
    module.header.as_mut().expect("SPIR-V header").bound = next;
    // naga is asked for debug names so the reads can be found by name;
    // nothing downstream wants them, and every other module is emitted
    // without them.
    module.debug_names.clear();
    let words = module.assemble();
    verify(&words, set, bindings, source_index, used);
    words
}

/// Re-reads the written module: it must parse, and declare exactly the
/// required input attachment reads where the executor binds them. The pass edits
/// a binary interface that nothing else in the build would notice going
/// wrong.
fn verify(words: &[u32], set: u32, bindings: [u32; 2], source_index: u32, used: [bool; 2]) {
    let module = rspirv::dr::load_words(words).expect("the rewritten module parses");
    assert!(
        module
            .capabilities
            .iter()
            .any(|instruction| instruction.operands
                == [Operand::Capability(Capability::InputAttachment)]),
        "the rewritten module does not declare InputAttachment"
    );
    let decorations = |id: Word| -> Vec<(Decoration, u32)> {
        module
            .annotations
            .iter()
            .filter(|instruction| {
                instruction.class.opcode == Op::Decorate
                    && instruction.operands.first() == Some(&Operand::IdRef(id))
            })
            .filter_map(|instruction| {
                match (instruction.operands.get(1), instruction.operands.get(2)) {
                    (Some(&Operand::Decoration(d)), Some(&Operand::LiteralBit32(v))) => {
                        Some((d, v))
                    }
                    _ => None,
                }
            })
            .collect()
    };
    let subpass = module
        .types_global_values
        .iter()
        .filter(|instruction| instruction.class.opcode == Op::TypeImage)
        .filter(|instruction| {
            instruction.operands.get(1) == Some(&Operand::Dim(Dim::DimSubpassData))
        })
        .filter_map(|instruction| instruction.result_id)
        .count();
    assert_eq!(
        subpass, 1,
        "the rewritten module declares one subpass image type"
    );
    let mut found = Vec::new();
    for instruction in &module.types_global_values {
        if instruction.class.opcode != Op::Variable {
            continue;
        }
        let Some(id) = instruction.result_id else {
            continue;
        };
        let read = decorations(id);
        if read
            .iter()
            .any(|(d, _)| *d == Decoration::InputAttachmentIndex)
        {
            found.push((id, read));
        }
    }
    let expected_indices: Vec<_> = used
        .into_iter()
        .enumerate()
        .filter_map(|(i, used)| used.then_some(i))
        .collect();
    assert_eq!(
        found.len(),
        expected_indices.len(),
        "attachment read count differs"
    );
    for (index, (_, read)) in expected_indices.into_iter().zip(&found) {
        let expected = [
            (Decoration::DescriptorSet, set),
            (Decoration::Binding, bindings[index]),
            (
                Decoration::InputAttachmentIndex,
                if index == 0 { 0 } else { source_index },
            ),
        ];
        for pair in expected {
            assert!(
                read.contains(&pair),
                "an attachment read is missing {pair:?}"
            );
        }
    }
}

/// The id of the private global `name`.
fn read_global(module: &Module, name: &str) -> Option<Word> {
    let named: Vec<Word> = module
        .debug_names
        .iter()
        .filter(|instruction| {
            instruction.class.opcode == Op::Name
                && matches!(instruction.operands.get(1), Some(Operand::LiteralString(s)) if s == name)
        })
        .filter_map(|instruction| match instruction.operands.first() {
            Some(&Operand::IdRef(id)) => Some(id),
            _ => None,
        })
        .collect();
    assert!(named.len() <= 1, "duplicate attachment global {name}");
    let &id = named.first()?;
    let variable = module
        .types_global_values
        .iter()
        .find(|instruction| instruction.result_id == Some(id))
        .expect("the named id is a module-scope declaration");
    assert_eq!(
        variable.class.opcode,
        Op::Variable,
        "{name} is not a variable"
    );
    Some(id)
}

/// The type a variable's pointer type points to, which must be private
/// storage: the engine's attachment reads are not descriptors yet.
fn pointee(module: &Module, variable: Word) -> Word {
    let pointer = module
        .types_global_values
        .iter()
        .find(|instruction| instruction.result_id == Some(variable))
        .and_then(|instruction| instruction.result_type)
        .expect("a variable has a pointer type");
    let declaration = module
        .types_global_values
        .iter()
        .find(|instruction| instruction.result_id == Some(pointer))
        .expect("an unresolved pointer type");
    assert_eq!(
        declaration.operands.first(),
        Some(&Operand::StorageClass(StorageClass::Private)),
        "an attachment read global is not private storage"
    );
    match declaration.operands.get(1) {
        Some(&Operand::IdRef(id)) => id,
        _ => panic!("a pointer type without a pointee"),
    }
}

/// The scalar component of a vector type.
fn component(module: &Module, vector: Word) -> Word {
    let declaration = module
        .types_global_values
        .iter()
        .find(|instruction| instruction.result_id == Some(vector))
        .expect("an unresolved colour type");
    assert_eq!(
        declaration.class.opcode,
        Op::TypeVector,
        "the attachment read colour is not a vector"
    );
    match declaration.operands.first() {
        Some(&Operand::IdRef(id)) => id,
        _ => panic!("a vector type without a component type"),
    }
}

/// Replaces every load of `global` with a read of `image` at `origin`,
/// returning how many it replaced. Any other use of `global` panics: the
/// read must not survive as private storage.
fn replace_loads(
    module: &mut Module,
    global: Word,
    image: (Word, Word),
    colour: Word,
    origin: Word,
    next: &mut Word,
) -> usize {
    let (image, image_ty) = image;
    let mut replaced = 0;
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut instructions = Vec::with_capacity(block.instructions.len());
            for instruction in block.instructions.drain(..) {
                let loads = instruction.class.opcode == Op::Load
                    && instruction.operands.first() == Some(&Operand::IdRef(global));
                if !loads {
                    assert!(
                        !instruction.operands.contains(&Operand::IdRef(global)),
                        "an attachment read global is used by {:?}",
                        instruction.class.opcode
                    );
                    instructions.push(instruction);
                    continue;
                }
                let loaded = *next;
                *next += 1;
                instructions.push(Instruction::new(
                    Op::Load,
                    Some(image_ty),
                    Some(loaded),
                    vec![Operand::IdRef(image)],
                ));
                instructions.push(Instruction::new(
                    Op::ImageRead,
                    Some(colour),
                    instruction.result_id,
                    vec![Operand::IdRef(loaded), Operand::IdRef(origin)],
                ));
                replaced += 1;
            }
            block.instructions = instructions;
        }
    }
    replaced
}

/// The id of an existing identical declaration, or a fresh one appended
/// to the module. SPIR-V forbids declaring a non-aggregate type or a
/// constant twice, and naga's emission already holds most of what the
/// reads need.
fn intern(
    module: &mut Module,
    next: &mut Word,
    opcode: Op,
    result_type: Option<Word>,
    operands: Vec<Operand>,
) -> Word {
    let existing = module.types_global_values.iter().find(|instruction| {
        instruction.class.opcode == opcode
            && instruction.result_type == result_type
            && instruction.operands == operands
    });
    if let Some(instruction) = existing {
        return instruction
            .result_id
            .expect("a declaration has a result id");
    }
    let id = *next;
    *next += 1;
    module
        .types_global_values
        .push(Instruction::new(opcode, result_type, Some(id), operands));
    id
}
