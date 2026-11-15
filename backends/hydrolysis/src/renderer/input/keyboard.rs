// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use waterui_core::contract_violation;

/// What became of a key press once the framework finished with it.
///
/// The distinction that matters is `Consumed` vs `ForwardedToSurface`:
/// only a consumed press suppresses the paired `TextInput` that follows it
/// in the event queue (the web platform's `keydown` → `beforeinput` rule —
/// `preventDefault` on the press cancels the text). A press forwarded to an
/// embedded surface was *delivered*, not consumed: the surface owns its
/// key+text pair, exactly as `SurfaceInputEvent` documents, and decides
/// internally what the press meant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyPressOutcome {
    /// No handler, editor, or surface accepted the press.
    Ignored,
    /// A handler consumed the press — editing action, focus traversal, or
    /// an `OnKeyPress` ancestor reporting `KeyHandling::Handled`.
    Consumed,
    /// The press was forwarded to an embedded surface; its paired text is
    /// delivered too, as the surface contract.
    ForwardedToSurface,
}

/// One `OnKeyPress` ancestor scope a focused input's unconsumed keys bubble
/// into. The scope stack is pushed while the retained tree flushes the
/// `OnKeyPress` wrapper (outermost first), and a target snapshotting it keeps
/// the whole chain even after the frame that produced it is gone.
pub struct KeyHandlerScope {
    /// The environment the `.on_key_press` view was built under — the handler
    /// resolves `State`/`Use` extractors against it, extended with the press.
    pub(crate) env: Environment,
    pub(crate) handler: Rc<RefCell<OnKeyPress>>,
}

/// One link of the `OnKeyPress` scope chain: the innermost scope at a
/// registration point plus the rest of its ancestors. Pushing a scope
/// allocates a single node, and a target's snapshot of the chain is a single
/// `Rc` clone, so neither the walk nor the snapshot allocates per frame.
pub struct KeyHandlerNode {
    pub scope: KeyHandlerScope,
    pub(crate) parent: Option<Rc<Self>>,
}

/// The innermost chain node shared by `a` and `b` — the scopes enclosing
/// every registration the two chains were snapped from.
pub fn common_key_handler_scope(
    a: Option<Rc<KeyHandlerNode>>,
    b: Option<Rc<KeyHandlerNode>>,
) -> Option<Rc<KeyHandlerNode>> {
    fn depth(mut node: Option<Rc<KeyHandlerNode>>) -> usize {
        let mut depth = 0;
        // The while-let moves `node` into the pattern each pass, so a fresh
        // clone is the only way to rebind it — `clone_from` needs a live
        // target binding, which this loop does not have.
        #[allow(clippy::assigning_clones)]
        while let Some(link) = node {
            depth += 1;
            node = link.parent.clone();
        }
        depth
    }
    let mut a = a;
    let mut b = b;
    let mut a_depth = depth(a.clone());
    let mut b_depth = depth(b.clone());
    while a_depth > b_depth {
        a = a.and_then(|link| link.parent.clone());
        a_depth -= 1;
    }
    while b_depth > a_depth {
        b = b.and_then(|link| link.parent.clone());
        b_depth -= 1;
    }
    loop {
        match (a, b) {
            (Some(x), Some(y)) => {
                if Rc::ptr_eq(&x, &y) {
                    return Some(x);
                }
                // `a`/`b` are moved into the match pattern, so rebinding
                // them takes fresh clones — `clone_from` cannot run on a
                // moved binding.
                #[allow(clippy::assigning_clones)]
                {
                    a = x.parent.clone();
                    b = y.parent.clone();
                }
            }
            _ => return None,
        }
    }
}

impl<E: crate::text::TextEngine> SemanticCore<E> {
    pub fn handle_key_with_env(
        &mut self,
        key: &KeyCode,
        modifiers: Modifiers,
        env: &Environment,
    ) -> bool {
        // Callers without platform key data (tests, synthetic input) get a
        // `KeyPress` rebuilt from the `KeyCode` — the winit/semantic paths
        // carry real `logical_key`/`physical_code` and call
        // `handle_key_press` instead.
        let press = KeyPress::new(
            key.to_w3c_key(),
            keyboard_types::Code::Unidentified,
            modifiers.into(),
            false,
        );
        self.handle_key_press(key, modifiers, env, &press) != KeyPressOutcome::Ignored
    }

    /// A key press with its full platform identity: the focused target's
    /// editing first, then the `OnKeyPress` bubble chain, then an enclosing
    /// embedded surface.
    ///
    /// The [`KeyPressOutcome`] distinguishes a press a handler consumed
    /// (whose paired `KeyText` is then suppressed at dispatch) from one
    /// forwarded to an embedded surface — delivery is not consumption: the
    /// surface owns its key+text pair, exactly as `SurfaceInputEvent`
    /// documents.
    pub(crate) fn handle_key_press(
        &mut self,
        key: &KeyCode,
        modifiers: Modifiers,
        env: &Environment,
        press: &KeyPress,
    ) -> KeyPressOutcome {
        if self.handle_keyboard_key_down(key, press.key(), modifiers, env) {
            return KeyPressOutcome::Consumed;
        }
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        if self.handle_key(key, modifiers) {
            return KeyPressOutcome::Consumed;
        }
        self.bubble_key_press(press)
    }

    /// Offers an unconsumed key to the focused node's `OnKeyPress`
    /// ancestors, nearest first, then to the topmost embedded surface
    /// enclosing it. [`KeyPressOutcome::Consumed`] once some scope reports
    /// [`KeyHandling::Handled`]; [`KeyPressOutcome::ForwardedToSurface`] when
    /// a surface takes the key.
    ///
    /// The focused node is whatever `hit_test.keyboard_focus` names — a text
    /// input, a focusable control's press slot, or an embedded surface; with
    /// no focus at all the key still bubbles through the scopes enclosing
    /// every registration of the frame.
    fn bubble_key_press(&mut self, press: &KeyPress) -> KeyPressOutcome {
        let focused_key = self.hit_test.keyboard_focus.clone();
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        let (mut scopes, mut bubble_center) =
            self.text_editing
                .focused_target()
                .map_or((None, None), |target| {
                    (
                        Some(target.key_handlers.clone()),
                        Some(target.bounds.center()),
                    )
                });
        #[cfg(not(any(feature = "text-field", feature = "secure-field")))]
        let (mut scopes, mut bubble_center): (
            Option<Option<Rc<KeyHandlerNode>>>,
            Option<kurbo::Point>,
        ) = (None, None);
        if scopes.is_none()
            && let Some(key) = focused_key.as_ref()
        {
            if let Some(target) = self.hit_test.pointer_targets.iter().find(|target| {
                target
                    .press_slot
                    .as_ref()
                    .is_some_and(|slot| &slot.key == key)
            }) {
                scopes = Some(target.key_handlers.clone());
                bubble_center = Some(target.bounds.center());
            } else if let Some(target) = self
                .hit_test
                .embedded_input_targets
                .iter()
                .find(|target| &target.interaction_key == key)
            {
                scopes = Some(target.key_handlers.clone());
                bubble_center = Some(target.to_window_rect(target.local_bounds).center());
            } else {
                // The semantic walk emits no targets for the key to resolve
                // against — the chain recorded at the focus link stands in.
                #[cfg(feature = "accessibility")]
                {
                    scopes = self.accessibility.focus_key_handlers.get(key).cloned();
                }
            }
        }
        // `scopes` wraps a chain head: a target with no scopes is
        // `Some(None)`; no resolved target at all falls back to the
        // every-registration chain.
        let mut node = scopes.unwrap_or_else(|| self.hit_test.root_key_handlers.clone());
        // `node` is moved into the while-let pattern each pass, so a fresh
        // clone rebinds it — `clone_from` needs a live target binding.
        #[allow(clippy::assigning_clones)]
        while let Some(link) = node {
            let handler = Rc::clone(&link.scope.handler);
            let env = link.scope.env.extending(press.clone());
            let result = handler.borrow_mut().handle(&env);
            if result == KeyHandling::Handled {
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    key = ?press.key(),
                    "key consumed by an on_key_press ancestor"
                );
                return KeyPressOutcome::Consumed;
            }
            node = link.parent.clone();
        }
        // Nothing above the focused node consumed it: the embedding surface
        // under it (e.g. a terminal under a search overlay) gets the key next.
        if let Some(center) = bubble_center
            && let Some((index, _)) = self.topmost_embedded_target_at(center)
        {
            let embedded = self.hit_test.embedded_input_targets[index].clone();
            embedded.sink.key(&KeyDelivery {
                pressed: true,
                logical: press.key(),
                code: press.code(),
                repeat: press.repeat(),
                modifiers: Modifiers::from(press.modifiers()),
            });
            // The release belongs to the sink that saw the press, not to
            // whichever surface holds focus when it arrives.
            self.hit_test.bubbled_key_sinks.push(BubbledKeySink {
                logical: press.key().clone(),
                code: press.code(),
                modifiers: Modifiers::from(press.modifiers()),
                sink: embedded.sink,
            });
            // Forwarded, not consumed: the surface owns the key+text pair
            // and decides internally what the press meant, so its paired
            // `TextInput` is still delivered.
            return KeyPressOutcome::ForwardedToSurface;
        }
        KeyPressOutcome::Ignored
    }

    pub fn handle_key_release_with_env(&mut self, key: &KeyCode, env: &Environment) -> bool {
        self.handle_keyboard_key_up(key, env)
    }
}

impl<E: crate::text::TextEngine> SemanticCore<E> {
    /// Wires a `.focused(binding)` modifier to the single focusable target —
    /// a text field or an input surface — registered inside `scope`'s subtree:
    /// writes the binding onto that target, then applies the binding's value
    /// to the focused-input key. The retained equivalent of the flat lists'
    /// `[start..]` span: entries owned by cells inside `scope`'s subtree,
    /// hittable or not — a `.focused()` inside an unhittable subtree still saw
    /// the target on dev (the outer truncation ran after the wiring), and the
    /// post-flush validation releases whatever focus a dead chain took.
    /// Shared by the rendered flush and the semantic accessibility walk.
    pub(crate) fn wire_focused_target(
        &mut self,
        value: &waterui::component::focus::Focused,
        should_focus: bool,
        scope: &Rc<NodeCell<E>>,
    ) {
        // Whether the entry's owning cell is `scope` or a descendant of it.
        let in_scope = |owner: &std::rc::Weak<NodeCell<E>>| -> bool {
            let mut cell = owner.upgrade();
            while let Some(current) = cell {
                if Rc::ptr_eq(&current, scope) {
                    return true;
                }
                cell = current.parent();
            }
            false
        };
        // The retained registries own entries per recording cell — the
        // scan walks every live owner's bucket and keeps the cell it
        // found, so the write-back lands in the same bucket.
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        let mut text_hit = None;
        let mut embedded_hit = None;
        let mut count = 0usize;
        for cell in self.retained.owners() {
            let slot = cell.registrations.borrow();
            let Some(regs) = slot.as_ref() else {
                continue;
            };
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
            for (index, entry) in regs.text_input_targets.iter().enumerate() {
                if in_scope(&entry.owner) {
                    count += 1;
                    text_hit = Some((Rc::clone(&cell), index));
                }
            }
            for (index, entry) in regs.embedded_input_targets.iter().enumerate() {
                if in_scope(&entry.owner) {
                    count += 1;
                    embedded_hit = Some((Rc::clone(&cell), index));
                }
            }
        }
        if count != 1 {
            contract_violation!(
                "hydrolysis .focused() requires exactly one TextField or SecureField or input surface in the wrapped subtree, found {count}; the binding is not wired"
            );
            return;
        }
        if let Some((cell, index)) = embedded_hit {
            // The bucket borrow ends before the focus setters run: they
            // materialize the registries, which borrow the same `RefCell`.
            let target_key = {
                let mut slot = cell.registrations.borrow_mut();
                let target = &mut slot
                    .as_mut()
                    .expect("hydrolysis focused metadata missing embedded input bucket")
                    .embedded_input_targets[index]
                    .payload;
                if target.focus_binding.is_some() {
                    contract_violation!(
                        "hydrolysis does not allow multiple .focused() modifiers to target the same control; the later binding is not wired"
                    );
                    return;
                }
                target.focus_binding = Some(value.0.clone());
                target.interaction_key.clone()
            };

            if should_focus {
                self.set_focused_embedded_key(Some(target_key));
            } else if self.is_focused_embedded(&target_key) {
                self.set_focused_embedded_key(None);
            }
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
            return;
        }
        #[cfg(any(feature = "text-field", feature = "secure-field"))]
        {
            let (cell, index) =
                text_hit.expect("hydrolysis focused metadata missing registered text input target");
            let target_key = {
                let mut slot = cell.registrations.borrow_mut();
                let target = &mut slot
                    .as_mut()
                    .expect("hydrolysis focused metadata missing text input bucket")
                    .text_input_targets[index]
                    .payload;
                if target.focus_binding.is_some() {
                    contract_violation!(
                        "hydrolysis does not allow multiple .focused() modifiers to target the same control; the later binding is not wired"
                    );
                    return;
                }
                target.focus_binding = Some(value.0.clone());
                target.interaction_key.clone()
            };

            if should_focus {
                self.set_focused_text_input_key(Some(target_key));
            } else if self.text_editing.is_focused(&target_key) {
                self.set_focused_text_input_key(None);
            }
        }
    }
}
