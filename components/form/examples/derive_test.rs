//! Example demonstrating the `FormBuilder` derive macro.

use waterui::Binding;
use waterui_form::FormBuilder;

#[derive(FormBuilder, Default, Clone, Debug)]
#[allow(dead_code)] // This is just an example
struct UserForm {
    username: String,
    age: i32,
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
    let _form_view = UserForm::view(&binding);
    println!("Form view created successfully!");
    println!("FormBuilder derive macro is working!");
}
