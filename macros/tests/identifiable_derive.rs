//! Integration coverage for the `Identifiable` derive macro.

use waterui::Identifiable;

#[derive(Clone, Debug, Identifiable)]
struct Contact {
    #[id]
    id: u64,
    name: &'static str,
}

#[derive(Clone, Debug, Identifiable)]
struct Article<Key> {
    #[id]
    slug: Key,
    title: &'static str,
}

#[derive(Clone, Copy, Debug, Identifiable)]
struct ContactId(#[id] u64);

#[test]
fn derives_identity_from_the_id_field() {
    let contact = Contact {
        id: 42,
        name: "Ada",
    };

    assert_eq!(contact.id(), 42);
    assert_eq!(contact.name, "Ada");
}

#[test]
fn derives_identity_from_a_marked_generic_field() {
    let article = Article {
        slug: String::from("native-first"),
        title: "Native first",
    };

    assert_eq!(article.id(), "native-first");
    assert_eq!(article.title, "Native first");
}

#[test]
fn derives_identity_for_a_newtype() {
    let contact_id = ContactId(7);

    assert_eq!(contact_id.id(), 7);
}
