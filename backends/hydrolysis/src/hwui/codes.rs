/// Declares a module of wire codes and, under test, the table the
/// generated Kotlin mirrors (`hwui::kotlin`).
macro_rules! codes {
    (
        $(#[doc = $doc:literal])*
        $module:ident: $ty:ty => $object:literal {
            $( $(#[doc = $code_doc:literal])* $name:ident = $value:expr; )*
        }
    ) => {
        $(#[doc = $doc])*
        pub mod $module {
            $( $(#[doc = $code_doc])* pub const $name: $ty = $value; )*

            code_metadata! {
            /// The codes, for the generated Kotlin.
            pub const CODES: crate::hwui::kotlin::Codes<$ty> = crate::hwui::kotlin::Codes {
                object: $object,
                doc: &[$($doc),*],
                entries: &[$( (stringify!($name), $name, &[$($code_doc),*]) ),*],
            };
            }
        }
    };
}
