//! R8 keep rules a crate declares under `[package.metadata.waterui.android]`,
//! and their staging into the generated module's `proguard-rules.pro`.
//!
//! A crate whose Rust code reaches classes of a platform SDK through JNI —
//! by name, which R8 cannot see — keeps exactly those classes and members:
//!
//! ```toml
//! [[package.metadata.waterui.android.keep]]
//! class = "com.example.sdk.Entry"
//! members = ["public static void start(android.content.Context)"]
//! ```
//!
//! A declaration names one fully qualified class and the exact members to
//! keep; no members keeps the class itself only. Wildcards are rejected, so
//! a declaration can never keep a package or a whole class, and R8 still
//! removes everything the crate does not reach (Principle 5).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use askama::Template;
use serde::Deserialize;

/// A fully qualified Java class name: `.`-separated Java identifiers, `$`
/// for a nested class, in a named package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct JavaClass(String);

impl TryFrom<String> for JavaClass {
    type Error = String;

    fn try_from(class: String) -> Result<Self, Self::Error> {
        let segments: Vec<&str> = class.split('.').collect();
        if segments.len() < 2 || !segments.iter().all(|segment| is_java_identifier(segment)) {
            return Err(format!(
                "keep class `{class}` must be one fully qualified class such as `com.example.Entry`, without wildcards"
            ));
        }
        Ok(Self(class))
    }
}

impl fmt::Display for JavaClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One class member in R8's member syntax, naming exactly one field, method
/// or constructor: `<modifiers> <type> <name>` for a field,
/// `<modifiers> <type> <name>(<parameter types>)` for a method and
/// `<init>(<parameter types>)` for a constructor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct KeepMember(String);

impl TryFrom<String> for KeepMember {
    type Error = String;

    fn try_from(member: String) -> Result<Self, Self::Error> {
        if member_is_exact(&member) {
            Ok(Self(member))
        } else {
            Err(format!(
                "keep member `{member}` must name exactly one field, method or constructor, such as `public static void start(android.content.Context)`, without wildcards"
            ))
        }
    }
}

impl fmt::Display for KeepMember {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `segment` is a Java identifier, `$` included for nested classes.
fn is_java_identifier(segment: &str) -> bool {
    let mut chars = segment.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || matches!(first, '_' | '$'))
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
}

/// Whether `ty` is a Java type: a primitive, `void` or a qualified class
/// name, followed by any number of `[]`.
fn is_java_type(ty: &str) -> bool {
    let base = ty.trim_end_matches("[]");
    !base.is_empty() && base.split('.').all(is_java_identifier)
}

/// Whether `member` names exactly one member: every type and name is
/// spelled out, so no wildcard, `<methods>`/`<fields>` or `...` can match
/// more than it.
fn member_is_exact(member: &str) -> bool {
    let (head, parameters) = match member.split_once('(') {
        Some((head, rest)) => match rest.strip_suffix(')') {
            Some(parameters) => (head, Some(parameters)),
            None => return false,
        },
        None => (member, None),
    };
    if let Some(parameters) = parameters {
        let parameters = parameters.trim();
        if !parameters.is_empty()
            && !parameters
                .split(',')
                .all(|parameter| is_java_type(parameter.trim()))
        {
            return false;
        }
    }
    let words: Vec<&str> = head.split_whitespace().collect();
    match (words.as_slice(), parameters) {
        // A constructor carries no return type.
        ([.., "<init>"], Some(_)) => words[..words.len() - 1]
            .iter()
            .all(|modifier| is_modifier(modifier)),
        // A field or a method: modifiers, a type, a name.
        ([modifiers @ .., ty, name], _) => {
            modifiers.iter().all(|modifier| is_modifier(modifier))
                && is_java_type(ty)
                && is_java_identifier(name)
        }
        _ => false,
    }
}

/// Whether `word` is a modifier R8's member syntax accepts.
fn is_modifier(word: &str) -> bool {
    matches!(
        word,
        "public"
            | "private"
            | "protected"
            | "static"
            | "final"
            | "native"
            | "synchronized"
            | "abstract"
            | "volatile"
            | "transient"
    )
}

/// One `[[package.metadata.waterui.android.keep]]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct KeepRule {
    /// The class to keep.
    class: JavaClass,
    /// The class's members to keep; none keeps the class alone.
    #[serde(default)]
    members: Vec<KeepMember>,
}

/// The keep rules the whole dependency graph declares, keyed by class: two
/// crates keeping one class keep the union of their members.
#[derive(Debug, Default)]
pub struct KeepRules {
    classes: BTreeMap<JavaClass, BTreeSet<KeepMember>>,
}

impl KeepRules {
    /// Adds the rules one table declares.
    pub(super) fn merge(&mut self, rules: Vec<KeepRule>) {
        for rule in rules {
            self.classes
                .entry(rule.class)
                .or_default()
                .extend(rule.members);
        }
    }

    /// Whether no crate declares a keep rule.
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
}

/// The managed keep block of a module's `proguard-rules.pro`.
#[derive(Template)]
#[template(
    path = "src/templates/android_gradle/keep_rules.pro.tpl",
    escape = "none"
)]
struct KeepBlockTemplate<'a> {
    begin: &'a str,
    end: &'a str,
    /// Packages of staged Kotlin sources, each kept whole.
    packages: &'a BTreeSet<String>,
    /// Declared classes and the members each keeps.
    classes: &'a BTreeMap<JavaClass, BTreeSet<KeepMember>>,
}

impl KeepRules {
    /// The managed keep block between `begin` and `end`: the packages of
    /// staged Kotlin sources, kept whole, then the declared classes and
    /// members.
    pub(super) fn block(
        &self,
        begin: &str,
        end: &str,
        packages: &BTreeSet<String>,
    ) -> askama::Result<String> {
        KeepBlockTemplate {
            begin,
            end,
            packages,
            classes: &self.classes,
        }
        .render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(text: &str) -> Result<KeepMember, String> {
        KeepMember::try_from(text.to_owned())
    }

    fn class(text: &str) -> Result<JavaClass, String> {
        JavaClass::try_from(text.to_owned())
    }

    #[test]
    fn exact_classes_parse() {
        assert!(class("com.example.sdk.Entry").is_ok());
        assert!(class("com.example.Outer$Inner").is_ok());
    }

    #[test]
    fn a_class_wildcard_or_bare_name_is_rejected() {
        for text in [
            "com.example.*",
            "com.example.**",
            "Entry",
            "com..Entry",
            "com.example.En try",
        ] {
            assert!(class(text).is_err(), "{text}");
        }
    }

    #[test]
    fn exact_members_parse() {
        for text in [
            "public static void start(android.content.Context)",
            "void stop()",
            "int count",
            "<init>(android.content.Context, java.lang.String[])",
            "public <init>()",
            "private static final java.lang.String TAG",
        ] {
            assert!(member(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn a_member_wildcard_is_rejected() {
        for text in [
            "*",
            "<methods>",
            "<fields>",
            "<init>",
            "*** get*(...)",
            "void start(...)",
            "public * start()",
            "void start(**)",
            "void start(android.content.Context",
        ] {
            assert!(member(text).is_err(), "{text}");
        }
    }

    #[test]
    fn two_crates_keeping_one_class_keep_the_union() {
        let mut rules = KeepRules::default();
        let entry = || class("com.example.Entry").expect("class");
        rules.merge(vec![KeepRule {
            class: entry(),
            members: vec![member("void a()").expect("member")],
        }]);
        rules.merge(vec![KeepRule {
            class: entry(),
            members: vec![
                member("void b()").expect("member"),
                member("void a()").expect("member"),
            ],
        }]);
        let members: Vec<String> = rules.classes[&entry()]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(members, ["void a()", "void b()"]);
    }

    #[test]
    fn the_block_renders_package_and_member_keeps() {
        let mut rules = KeepRules::default();
        rules.merge(vec![
            KeepRule {
                class: class("com.example.Entry").expect("class"),
                members: vec![
                    member("public static void start(android.content.Context)").expect("member"),
                ],
            },
            KeepRule {
                class: class("com.example.Marker").expect("class"),
                members: Vec::new(),
            },
        ]);
        let packages = BTreeSet::from(["waterkit.haptic".to_owned()]);
        let rendered = rules.block("# begin", "# end", &packages).expect("render");
        assert_eq!(
            rendered,
            "# begin\n\
             -keep class waterkit.haptic.** { *; }\n\
             -keep class com.example.Entry {\n    public static void start(android.content.Context);\n}\n\
             -keep class com.example.Marker\n\
             # end"
        );
    }
}
