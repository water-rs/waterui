//! Example demonstrating the `#[form]` attribute macro.

use waterui_core::Binding;
use waterui_form::{FormBuilder, form};

/// User profile form for testing the #[form] macro
#[form]
#[allow(dead_code)] // This is just an example
struct UserProfile {
    /// User's display name
    username: String,
    /// User's age
    age: i32,
    /// Email notifications enabled
    remember_me: bool,
    /// Profile completion (0.0 to 1.0)
    completion: f64,
}

fn main() {
    println!("Testing #[form] attribute macro...");

    // Create a form instance - all required traits are auto-derived
    let user_profile = UserProfile::default();
    println!("Created form: {user_profile:?}");

    // Test Clone trait
    let cloned_profile = user_profile.clone();
    println!("Cloned form: {cloned_profile:?}");

    // Create a binding using the container method
    let binding = Binding::container(user_profile);
    println!("Created binding");

    // Create the form view using the derived FormBuilder implementation
    let _form_view = form(&binding);
    println!("Form view created successfully!");

    // Test nami Project trait for reactive state management
    let projected = binding.project();
    println!("Created projected bindings for reactive updates");

    // Show that we can access individual field bindings
    projected.username.set("John Doe".to_string());
    projected.age.set(25);
    projected.completion.set(0.75);

    let updated_form = binding.get();
    println!("Updated form via projected bindings: {updated_form:?}");

    // Test serde serialization if feature is enabled
    #[cfg(feature = "serde")]
    {
        use serde_json;
        let json = serde_json::to_string(&updated_form).expect("Failed to serialize");
        println!("Serialized form: {json}");

        let deserialized: UserProfile = serde_json::from_str(&json).expect("Failed to deserialize");
        println!("Deserialized form: {deserialized:?}");
    }

    println!("#[form] macro is working perfectly!");
    println!("Auto-derived traits: Default, Clone, Debug, FormBuilder, Project");
    #[cfg(feature = "serde")]
    println!("Auto-derived serde traits: Serialize, Deserialize");
}
