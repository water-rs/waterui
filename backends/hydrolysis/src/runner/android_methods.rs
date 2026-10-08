//! The native→`HydrolysisSession` callback contract.
//!
//! Every Kotlin method the runner calls by name through JNI is declared
//! once here — a name and a signature in [`HOST_METHODS`], keyed by
//! [`HostMethodId`]. Session creation resolves each entry with
//! `GetMethodID` and caches the ids, so a host method R8 stripped or
//! renamed fails the create, not the first call that reaches for it. The
//! Kotlin side keeps the same set under `@CalledFromNative`; the host-side
//! test asserts the two agree on name and signature.

/// Positions in [`HOST_METHODS`], in the same order — the only handle a
/// call site names a host callback by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostMethodId {
    RequestRedraw,
    SoftInput,
    AccessibilityTreeChanged,
    PlatformViewsChanged,
    FatalError,
    CloseRequested,
    EditingState,
    CursorAnchorInfo,
    BackAvailable,
}

impl HostMethodId {
    /// Every id, in table order — the per-variant `dead_code` exemption for
    /// non-Android test builds and the test's order check both read it.
    pub const ALL: [Self; 9] = [
        Self::RequestRedraw,
        Self::SoftInput,
        Self::AccessibilityTreeChanged,
        Self::PlatformViewsChanged,
        Self::FatalError,
        Self::CloseRequested,
        Self::EditingState,
        Self::CursorAnchorInfo,
        Self::BackAvailable,
    ];
}

/// One `HydrolysisSession` method the native side calls: its JNI name and
/// signature, resolved into a cached `jmethodID` once per session.
pub struct HostMethod {
    pub name: &'static str,
    pub signature: &'static str,
}

/// The single declaration of the contract, in `HostMethodId` order. No
/// name or signature literal appears at a call site.
pub const HOST_METHODS: &[HostMethod] = &[
    HostMethod {
        name: "onNativeRequestRedraw",
        signature: "()V",
    },
    HostMethod {
        name: "onNativeSoftInput",
        signature: "(Z)V",
    },
    HostMethod {
        name: "onNativeAccessibilityTreeChanged",
        signature: "(Ljava/lang/String;)V",
    },
    HostMethod {
        name: "onNativePlatformViewsChanged",
        signature: "()V",
    },
    HostMethod {
        name: "onNativeFatalError",
        signature: "(Ljava/lang/String;)V",
    },
    HostMethod {
        name: "onNativeCloseRequested",
        signature: "()V",
    },
    HostMethod {
        name: "onNativeEditingState",
        signature: "(Ljava/lang/String;)V",
    },
    HostMethod {
        name: "onNativeCursorAnchorInfo",
        signature: "(Ljava/lang/String;)V",
    },
    HostMethod {
        name: "onNativeBackAvailable",
        signature: "(Z)V",
    },
];

const _: () = assert!(
    HOST_METHODS.len() == HostMethodId::ALL.len(),
    "HOST_METHODS and HostMethodId disagree"
);

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{HOST_METHODS, HostMethodId};

    /// The compiled `HydrolysisSession.class` lives in a Gradle build
    /// output, which `cargo nextest` does not run — the agreement check
    /// therefore reads the Kotlin source, the test-only comparison the
    /// design calls for, and `include_str!` keeps a moved file a compile
    /// error rather than a skip.
    const SESSION_KT: &str = include_str!(
        "../../android/host/src/main/java/dev/waterui/hydrolysis/HydrolysisSession.kt"
    );
    const ANNOTATION_KT: &str =
        include_str!("../../android/host/src/main/java/dev/waterui/hydrolysis/CalledFromNative.kt");

    /// The Kotlin parameter types the JNI table knows how to spell; an
    /// annotated method using any other type fails here loudly rather than
    /// producing a signature nothing on the JVM matches.
    fn kotlin_type_to_jni(kotlin: &str, method: &str) -> &'static str {
        match kotlin {
            "Boolean" => "Z",
            "String" => "Ljava/lang/String;",
            other => {
                panic!("@CalledFromNative method {method} takes an unmapped Kotlin type {other}")
            }
        }
    }

    /// Every `fun name(params)` directly preceded by `@CalledFromNative`,
    /// as `name → (params)V`.
    fn annotated_methods(source: &str) -> BTreeMap<String, String> {
        let mut methods = BTreeMap::new();
        let mut annotated = false;
        for line in source.lines() {
            let line = line.trim();
            if line.contains("@CalledFromNative") {
                annotated = true;
                continue;
            }
            if !annotated {
                continue;
            }
            annotated = false;
            let Some(rest) = line.strip_prefix("fun ") else {
                continue;
            };
            let name = rest.split('(').next().unwrap().to_owned();
            let params = rest
                .split_once('(')
                .and_then(|(_, tail)| tail.split_once(')'))
                .map(|(params, _)| params)
                .unwrap_or_else(|| panic!("unparseable @CalledFromNative method line: {rest}"));
            let signature = params
                .split(',')
                .filter(|param| !param.trim().is_empty())
                .map(|param| {
                    let (_, ty) = param.split_once(':').unwrap_or_else(|| {
                        panic!("@CalledFromNative parameter {param} has no type")
                    });
                    kotlin_type_to_jni(ty.trim(), &name)
                })
                .fold("(".to_owned(), |mut acc, ty| {
                    acc.push_str(ty);
                    acc
                })
                + ")V";
            methods.insert(name, signature);
        }
        methods
    }

    #[test]
    fn the_table_and_the_annotated_kotlin_methods_agree() {
        // Index order is part of the contract: `HostMethodId` names are the
        // table's `onNative` suffixes, so a reorder on either side is a
        // mismatch here, not a misdirected call on device.
        for (id, method) in HostMethodId::ALL.iter().zip(HOST_METHODS.iter()) {
            assert_eq!(
                method.name,
                format!("onNative{id:?}"),
                "HOST_METHODS must stay in HostMethodId order"
            );
        }

        let annotated = annotated_methods(SESSION_KT);
        let declared: BTreeMap<String, String> = HOST_METHODS
            .iter()
            .map(|method| (method.name.to_owned(), method.signature.to_owned()))
            .collect();
        assert_eq!(
            declared, annotated,
            "every HOST_METHODS entry needs a matching @CalledFromNative method on \
             HydrolysisSession, and vice versa"
        );
    }

    #[test]
    fn the_keep_annotation_survives_into_the_class_file() {
        // R8 matches the keep rule against annotation entries in the class
        // file — SOURCE retention would already be gone by then.
        assert!(
            ANNOTATION_KT.contains("@Retention(AnnotationRetention.BINARY)"),
            "@CalledFromNative must have BINARY retention for the consumer keep rule"
        );
    }
}
