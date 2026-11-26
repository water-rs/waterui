//! Reactive focus metadata shared by views and backends.

use nami::{Binding, Signal};
use waterui_core::metadata::MetadataKey;

/// A struct that represents a focused state based on a binding to a boolean value.
#[derive(Debug, Clone)]
pub struct Focused(pub Binding<bool>);

impl MetadataKey for Focused {}

impl Focused {
    /// Creates a new `Focused` instance based on an optional value binding.
    ///
    /// This function creates a binding that is true when the provided `value` binding
    /// contains a value that equals the provided `equals` parameter.
    pub fn new<T: 'static + Eq + Clone>(value: &Binding<Option<T>>, equals: T) -> Self {
        Self(Binding::mapping(
            value,
            {
                let equals = equals.clone();
                move |value| value.as_ref().is_some_and(|value| value == &equals)
            },
            move |binding, is_focused| {
                if is_focused {
                    binding.set(Some(equals.clone()));
                    return;
                }

                if binding
                    .snapshot()
                    .as_ref()
                    .is_some_and(|current| *current == equals)
                {
                    binding.set(None);
                }
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use nami::{Binding, Signal};

    use super::Focused;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Username,
        Password,
    }

    #[test]
    fn clearing_matching_focus_sets_source_to_none() {
        let source = Binding::new(Some(Field::Username));
        let focused = Focused::new(&source, Field::Username);

        focused.0.set(false);

        assert_eq!(source.snapshot(), None);
    }

    #[test]
    fn clearing_other_field_focus_is_ignored() {
        let source = Binding::new(Some(Field::Password));
        let focused = Focused::new(&source, Field::Username);

        focused.0.set(false);

        assert_eq!(source.snapshot(), Some(Field::Password));
    }

    #[test]
    fn reads_true_only_while_source_holds_the_matching_value() {
        let source = Binding::new(Some(Field::Username));
        let focused = Focused::new(&source, Field::Username);

        assert!(focused.0.snapshot());
        source.set(Some(Field::Password));
        assert!(!focused.0.snapshot());
        source.set(None);
        assert!(!focused.0.snapshot());
    }

    #[test]
    fn setting_focus_writes_the_matching_value() {
        let source = Binding::new(None::<Field>);
        let focused = Focused::new(&source, Field::Password);

        focused.0.set(true);

        assert_eq!(source.snapshot(), Some(Field::Password));
    }

    #[test]
    fn focusing_one_field_unfocuses_the_other() {
        let source = Binding::new(Some(Field::Username));
        let username = Focused::new(&source, Field::Username);
        let password = Focused::new(&source, Field::Password);

        password.0.set(true);

        assert_eq!(source.snapshot(), Some(Field::Password));
        assert!(!username.0.snapshot());
        assert!(password.0.snapshot());
    }

    #[test]
    fn clearing_focus_on_empty_source_is_a_no_op() {
        let source = Binding::new(None::<Field>);
        let focused = Focused::new(&source, Field::Username);

        focused.0.set(false);

        assert_eq!(source.snapshot(), None);
    }

    #[test]
    fn source_changes_reach_focused_watchers() {
        let source = Binding::new(Some(Field::Username));
        let focused = Focused::new(&source, Field::Username);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let _guard = focused.0.watch({
            let seen = Rc::clone(&seen);
            move |ctx| seen.borrow_mut().push(ctx.into_value())
        });

        source.set(Some(Field::Password));
        source.set(None);
        source.set(Some(Field::Username));

        assert_eq!(*seen.borrow(), vec![false, false, true]);
    }
}
