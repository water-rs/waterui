//! A container group's resolution (water-rs/waterui#2834): what the
//! theme's [`ContainerRecipe`] derives from all members of a
//! `.material_group()` scope whose shader registered one.
//!
//! Each member keeps its own recorded shape and effect live and binds the
//! group's effect for it through a [`Binding`] the group's resolution sets.
//! The resolution watches every member's shape and effect and re-runs
//! [`ContainerRecipe::effects`] over the current members when any of them
//! changes, so a change reaches the bound samples with no re-recording and
//! no structural recompute.

use std::cell::RefCell;
use std::rc::Rc;

use cherenkov_record::{
    CaptureClass, ContainerRecipe, GroupMember, MaterialEffect, MaterialRegistry, MaterialShader,
    ShapeData, SharedLive,
};
use waterui_core::contract_violation;

/// A container member's own terms: what its recording declared, and the
/// binding its sample reads its effect from — the group's resolution sets
/// it. Held in the member's payload, so a re-recording replaces it whole.
#[derive(Clone, Debug)]
pub struct ContainerMember {
    /// The member's live shape, in its recording's logical coordinates.
    pub shape: SharedLive<ShapeData>,
    /// The effect the member recorded for itself standing alone.
    pub effect: SharedLive<MaterialEffect>,
    /// The capture class the member recorded.
    pub capture: CaptureClass,
    /// The effect the member's sample runs: set by the resolution.
    pub binding: nami::Binding<MaterialEffect>,
}

/// One member as the resolution watches it: the current values the
/// recipe reads.
struct Watched {
    shape: ShapeData,
    capture: CaptureClass,
    effect: MaterialEffect,
}

/// The state the watchers of one resolution share.
struct State {
    registry: Rc<MaterialRegistry>,
    shader: MaterialShader,
    /// Each member's binding, in the order of `members`: set with no
    /// borrow of `members` held, since a set runs the watchers of the
    /// sample the binding feeds.
    bindings: Vec<nami::Binding<MaterialEffect>>,
    members: RefCell<Vec<Watched>>,
}

/// `members` as the recipe sees them.
fn seen(members: &[Watched]) -> Vec<GroupMember<'_>> {
    members
        .iter()
        .map(|member| GroupMember {
            shape: &member.shape,
            capture: member.capture,
            effect: &member.effect,
        })
        .collect()
}

/// Re-runs the recipe over `members` and returns its effects, or `None`
/// after a reported violation when it returns the wrong count. The recipe
/// is registered for the key the group was built under.
fn effects_of(
    recipe: &dyn ContainerRecipe,
    shader: MaterialShader,
    members: &[Watched],
) -> Option<Vec<MaterialEffect>> {
    let effects = recipe.effects(&seen(members));
    if effects.len() != members.len() {
        contract_violation!(
            "hydrolysis materials: the container recipe of shader {shader:?} returned {} effects \
             for {} members; every member keeps its last effect",
            effects.len(),
            members.len(),
        );
        return None;
    }
    Some(effects)
}

/// The effect a lone `member` gets from `recipe`: the first value of its
/// binding, before its group resolves it. A recipe that breaks its contract
/// is reported and the member's own effect stands.
pub fn solo_effect(
    recipe: &dyn ContainerRecipe,
    shader: MaterialShader,
    shape: &ShapeData,
    capture: CaptureClass,
    own: &MaterialEffect,
) -> MaterialEffect {
    let alone = [Watched {
        shape: shape.clone(),
        capture,
        effect: own.clone(),
    }];
    effects_of(recipe, shader, &alone)
        .and_then(|effects| effects.into_iter().next())
        .unwrap_or_else(|| own.clone())
}

impl State {
    /// Re-runs the recipe over the current members and sets every binding.
    fn resolve(&self) {
        let Some(recipe) = self.registry.container_recipe(self.shader) else {
            return;
        };
        let effects = effects_of(recipe, self.shader, &self.members.borrow());
        let Some(effects) = effects else {
            return;
        };
        for (binding, effect) in self.bindings.iter().zip(effects) {
            binding.set(effect);
        }
    }
}

/// One container group's live resolution: the watches that re-run the
/// recipe when a member's shape or effect changes. Dropping it ends them.
pub struct Resolution {
    state: Rc<State>,
    _guards: Vec<cherenkov_record::Binding>,
}

impl Resolution {
    /// Starts watching `members` under the recipe registered for
    /// `shader`, from the values their shapes and effects hold now. No
    /// binding is set until [`resolve`](Self::resolve) or a change.
    pub fn start<'a>(
        registry: &Rc<MaterialRegistry>,
        shader: MaterialShader,
        members: impl IntoIterator<Item = &'a ContainerMember>,
    ) -> Self {
        let members: Vec<&ContainerMember> = members.into_iter().collect();
        let watched = members
            .iter()
            .map(|member| Watched {
                shape: member.shape.rebound().value().clone(),
                capture: member.capture,
                effect: member.effect.rebound().value().clone(),
            })
            .collect();
        let state = Rc::new(State {
            registry: Rc::clone(registry),
            shader,
            bindings: members
                .iter()
                .map(|member| member.binding.clone())
                .collect(),
            members: RefCell::new(watched),
        });
        let mut guards = Vec::new();
        for (index, member) in members.into_iter().enumerate() {
            let (_, guard) = member.shape.clone().watch({
                let state = Rc::clone(&state);
                move |context| {
                    state.members.borrow_mut()[index].shape = context.into_value();
                    state.resolve();
                }
            });
            guards.extend(guard);
            let (_, guard) = member.effect.clone().watch({
                let state = Rc::clone(&state);
                move |context| {
                    state.members.borrow_mut()[index].effect = context.into_value();
                    state.resolve();
                }
            });
            guards.extend(guard);
        }
        Self {
            state,
            _guards: guards,
        }
    }

    /// The capture class `recipe` resolves for the members now.
    pub fn capture(&self, recipe: &dyn ContainerRecipe) -> CaptureClass {
        recipe.capture(&seen(&self.state.members.borrow()))
    }

    /// Re-runs the recipe over the current members and sets every binding.
    pub fn resolve(&self) {
        self.state.resolve();
    }
}
