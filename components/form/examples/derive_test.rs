//! Example demonstrating the `FormBuilder` derive macro.

use waterui::{Binding, component::form::FormBuilder};
use waterui_form::form;

#[form]
#[allow(dead_code)] // This is just an example
pub struct UserForm {
    /// The user's username
    username: String,
    /// The user's age
    age: i32,
    /// Whether to remember the user
    remember_me: bool,
}

fn main() {
    println!("Testing FormBuilder derive macro...");

    // Create a form instance
    let user_form = UserForm::default();
    println!("Created form: {user_form:?}");

    // Create a binding using the container method
    let binding = Binding::container(user_form);
    println!("Created binding");

    // Create the form view using the derived implementation
    let _form_view = form(&binding);
    println!("Form view created successfully!");
    println!("FormBuilder derive macro is working!");
}
