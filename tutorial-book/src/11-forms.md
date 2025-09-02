# Form Components

Form components are essential for user input and data collection. WaterUI provides comprehensive form controls with validation, accessibility, and reactive state management.

## Basic Form Controls

### Text Inputs

```rust,ignore
fn text_input_demo() -> impl View {
    let first_name = binding(String::new());
    let last_name = binding(String::new());
    let email = binding(String::new());
    let bio = binding(String::new());
    
    form([
        text_field(first_name.clone())
            .label("First Name")
            .placeholder("Enter your first name")
            .required(true),
            
        text_field(last_name.clone())
            .label("Last Name")
            .placeholder("Enter your last name")
            .required(true),
            
        text_field(email.clone())
            .label("Email")
            .placeholder("your@email.com")
            .input_type(InputType::Email)
            .required(true),
            
        text_area(bio.clone())
            .label("Bio")
            .placeholder("Tell us about yourself...")
            .rows(4)
            .max_length(500),
            
        text!("Character count: {}/500", s!(bio.len()))
            .font_size(12.0)
            .color(Color::secondary()),
    ))
    .spacing(15.0)
}
```

### Selection Controls

```rust,ignore
fn selection_controls_demo() -> impl View {
    let country = binding("US".to_string());
    let interests = binding(HashSet::<String>::new());
    let subscription_type = binding(SubscriptionType::Basic);
    let notifications_enabled = binding(true);
    
    form([
        // Dropdown/Picker
        picker(country.clone(), vec![
            ("US", "United States"),
            ("CA", "Canada"),
            ("UK", "United Kingdom"),
            ("DE", "Germany"),
            ("FR", "France"),
        ))
        .label("Country")
        .required(true),
        
        // Multi-select checkboxes
        checkbox_group(interests.clone(), vec![
            ("tech", "Technology"),
            ("design", "Design"),
            ("music", "Music"),
            ("sports", "Sports"),
            ("travel", "Travel"),
        ))
        .label("Interests")
        .columns(2),
        
        // Radio buttons
        radio_group(subscription_type.clone(), vec![
            (SubscriptionType::Basic, "Basic - Free"),
            (SubscriptionType::Pro, "Pro - $9.99/month"),
            (SubscriptionType::Enterprise, "Enterprise - Contact sales"),
        ))
        .label("Subscription Type")
        .required(true),
        
        // Toggle switch
        toggle(notifications_enabled.clone())
            .label("Enable notifications"),
            
        // Display selections
        text!("Selected country: {}", country),
        text!("Interests: {}", s!(interests.iter().collect::<Vec<_>>().join(", "))),
        text!("Subscription: {:?}", subscription_type),
        text!("Notifications: {}", s!(if notifications_enabled { "On" } else { "Off" })),
    ))
    .spacing(15.0)
}

#[derive(Clone, Debug, PartialEq)]
enum SubscriptionType {
    Basic,
    Pro,
    Enterprise,
}
```

### Numeric Inputs

```rust,ignore
fn numeric_input_demo() -> impl View {
    let age = binding(25);
    let salary = binding(50000.0);
    let rating = binding(4.0);
    let quantity = binding(1);
    
    form([
        // Number field
        number_field(age.clone())
            .label("Age")
            .min(13)
            .max(120)
            .required(true),
            
        // Currency field
        currency_field(salary.clone())
            .label("Annual Salary")
            .currency_code("USD")
            .min(0.0),
            
        // Slider for rating
        vstack((
            text("Rating"),
            slider(s!(rating / 5.0))
                .on_change({
                    let rating = rating.clone();
                    move |value| rating.set(value * 5.0)
                }),
            text!("Rating: {:.1}/5.0", rating),
        ))
        .spacing(5.0),
        
        // Stepper for quantity
        stepper(quantity.clone())
            .label("Quantity")
            .min(1)
            .max(99)
            .step(1),
            
        text!("Values: Age={}, Salary=${:.2}, Rating={:.1}, Quantity={}", 
              age, salary, rating, quantity),
    ))
    .spacing(15.0)
}
```

## Advanced Form Components

### Date and Time Inputs

```rust,ignore
fn date_time_demo() -> impl View {
    let birth_date = binding(Date::today());
    let appointment_datetime = binding(DateTime::now());
    let reminder_time = binding(Time::new(9, 0));
    
    form([
        date_picker(birth_date.clone())
            .label("Date of Birth")
            .max_date(Date::today())
            .required(true),
            
        datetime_picker(appointment_datetime.clone())
            .label("Appointment")
            .min_datetime(DateTime::now())
            .format(DateTimeFormat::Full),
            
        time_picker(reminder_time.clone())
            .label("Daily Reminder")
            .format(TimeFormat::TwentyFourHour),
            
        text!("Birth Date: {}", birth_date),
        text!("Appointment: {}", appointment_datetime),
        text!("Reminder: {}", reminder_time),
    ))
    .spacing(15.0)
}
```

### File Upload

```rust,ignore
fn file_upload_demo() -> impl View {
    let uploaded_files = binding(Vec::<FileInfo>::new());
    let is_uploading = binding(false);
    let upload_progress = binding(0.0);
    
    form([
        file_upload()
            .label("Upload Documents")
            .accept(&[".pdf", ".doc", ".docx", ".txt"))
            .multiple(true)
            .max_size(10 * 1024 * 1024) // 10MB
            .on_files_selected({
                let uploaded_files = uploaded_files.clone();
                let is_uploading = is_uploading.clone();
                let upload_progress = upload_progress.clone();
                move |files| {
                    is_uploading.set(true);
                    upload_progress.set(0.0);
                    
                    let uploaded_files = uploaded_files.clone();
                    let is_uploading = is_uploading.clone();
                    let upload_progress = upload_progress.clone();
                    
                    task::spawn(async move {
                        for (i, file) in files.iter().enumerate() {
                            // Simulate upload progress
                            for p in 0..=100 {
                                upload_progress.set(p as f64 / 100.0);
                                task::sleep(Duration::from_millis(10)).await;
                            }
                            
                            uploaded_files.update(|files| {
                                files.push(file.clone());
                                files
                            });
                        }
                        
                        is_uploading.set(false);
                        upload_progress.set(0.0);
                    });
                }
            }),
            
        s!(if is_uploading {
            Some(
                vstack((
                    text("Uploading..."),
                    progress_bar(upload_progress.get()),
                    text!("Progress: {:.0}%", s!(upload_progress * 100.0)),
                ))
                .spacing(5.0)
            )
        } else {
            None
        }),
        
        // File list
        vstack(
            uploaded_files.signal().map(|files| {
                files.into_iter().map(|file| {
                    file_item(file)
                })
            })
        ),
    ))
    .spacing(15.0)
}

#[derive(Clone)]
struct FileInfo {
    name: String,
    size: u64,
    mime_type: String,
}

fn file_item(file: FileInfo) -> impl View {
    hstack((
        text("📄"),
        vstack((
            text(&file.name).font_weight(FontWeight::Medium),
            text!("{} KB", file.size / 1024)
                .font_size(12.0)
                .color(Color::secondary()),
        ))
        .flex(1),
        button("Remove")
            .style(ButtonStyle::Destructive)
            .action(move |_| {
                // Remove file logic
            }),
    ))
    .padding(10.0)
    .background(Color::light_gray())
    .corner_radius(5.0)
    .spacing(10.0)
}
```

## Form Validation

### Field-Level Validation

```rust,ignore
fn validation_demo() -> impl View {
    let email = binding(String::new());
    let password = binding(String::new());
    let confirm_password = binding(String::new());
    let age = binding(0);
    
    // Validation rules
    let email_valid = s!(validate_email(&email));
    let password_valid = s!(validate_password(&password));
    let passwords_match = s!(password == confirm_password && !confirm_password.is_empty());
    let age_valid = s!(age >= 13 && age <= 120);
    
    form([
        validated_field(
            text_field(email.clone())
                .label("Email")
                .placeholder("your@email.com")
                .required(true),
            email_valid.clone(),
            "Please enter a valid email address"
        ),
        
        validated_field(
            secure_field(password.clone())
                .label("Password")
                .placeholder("Enter password")
                .required(true),
            password_valid.clone(),
            "Password must be at least 8 characters with uppercase, lowercase, and number"
        ),
        
        validated_field(
            secure_field(confirm_password.clone())
                .label("Confirm Password")
                .placeholder("Confirm password")
                .required(true),
            passwords_match.clone(),
            "Passwords do not match"
        ),
        
        validated_field(
            number_field(age.clone())
                .label("Age")
                .min(13)
                .max(120)
                .required(true),
            age_valid.clone(),
            "Age must be between 13 and 120"
        ),
        
        button("Submit")
            .disabled(s!(!(email_valid && password_valid && passwords_match && age_valid)))
            .action(move |_| {
                println!("Form submitted successfully!");
            }),
    ))
    .spacing(15.0)
}

fn validated_field<V: View>(
    field: V,
    is_valid: Signal<bool>,
    error_message: &str,
) -> impl View {
    let error_msg = error_message.to_string();
    
    vstack((
        field,
        s!(if is_valid {
            None
        } else {
            Some(
                text(&error_msg)
                    .color(Color::red())
                    .font_size(12.0)
            )
        }),
    ))
    .spacing(5.0)
}

fn validate_email(email: &str) -> bool {
    email.contains('@') && email.contains('.') && email.len() > 5
}

fn validate_password(password: &str) -> bool {
    password.len() >= 8
        && password.chars().any(|c| c.is_uppercase())
        && password.chars().any(|c| c.is_lowercase())
        && password.chars().any(|c| c.is_numeric())
}
```

### Form-Level Validation

```rust,ignore
fn form_validation_demo() -> impl View {
    let form_data = binding(UserRegistrationForm::default());
    let validation_errors = binding(Vec::<String>::new());
    let is_submitting = binding(false);
    
    form([
        text_field(s!(form_data.first_name.clone()))
            .label("First Name")
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.first_name = value;
                        data
                    });
                }
            }),
            
        text_field(s!(form_data.last_name.clone()))
            .label("Last Name")
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.last_name = value;
                        data
                    });
                }
            }),
            
        text_field(s!(form_data.email.clone()))
            .label("Email")
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.email = value;
                        data
                    });
                }
            }),
            
        // Validation errors display
        s!(if validation_errors.is_empty() {
            None
        } else {
            Some(
                vstack(
                    validation_errors.signal().map(|errors| {
                        errors.into_iter().map(|error| {
                            text(&error)
                                .color(Color::red())
                                .font_size(14.0)
                        })
                    })
                )
                .padding(10.0)
                .background(Color::red().opacity(0.1))
                .corner_radius(5.0)
            )
        }),
        
        button("Register")
            .disabled(is_submitting.get())
            .action({
                let form_data = form_data.clone();
                let validation_errors = validation_errors.clone();
                let is_submitting = is_submitting.clone();
                move |_| {
                    let data = form_data.get();
                    let errors = validate_registration_form(&data);
                    
                    validation_errors.set(errors.clone());
                    
                    if errors.is_empty() {
                        is_submitting.set(true);
                        
                        let is_submitting = is_submitting.clone();
                        task::spawn(async move {
                            // Simulate API call
                            task::sleep(Duration::from_secs(2)).await;
                            is_submitting.set(false);
                            println!("Registration successful!");
                        });
                    }
                }
            }),
    ))
    .spacing(15.0)
}

#[derive(Clone, Default)]
struct UserRegistrationForm {
    first_name: String,
    last_name: String,
    email: String,
}

fn validate_registration_form(form: &UserRegistrationForm) -> Vec<String> {
    let mut errors = Vec::new();
    
    if form.first_name.trim().is_empty() {
        errors.push("First name is required".to_string());
    }
    
    if form.last_name.trim().is_empty() {
        errors.push("Last name is required".to_string());
    }
    
    if form.email.trim().is_empty() {
        errors.push("Email is required".to_string());
    } else if !validate_email(&form.email) {
        errors.push("Please enter a valid email address".to_string());
    }
    
    errors
}
```

## Complex Form Layouts

### Multi-Step Forms

```rust,ignore
fn multi_step_form_demo() -> impl View {
    let current_step = binding(0);
    let form_data = binding(CompleteFormData::default());
    
    let total_steps = 3;
    
    vstack((
        // Progress indicator
        step_progress_indicator(current_step.get(), total_steps),
        
        // Form content
        s!(match current_step {
            0 => Some(personal_info_step(form_data.clone())),
            1 => Some(contact_info_step(form_data.clone())),
            2 => Some(preferences_step(form_data.clone())),
            _ => None,
        }),
        
        // Navigation buttons
        form_navigation(current_step.clone(), total_steps),
    ))
    .spacing(20.0)
}

fn step_progress_indicator(current: usize, total: usize) -> impl View {
    hstack(
        (0..total).map(|step| {
            circle()
                .width(30.0)
                .height(30.0)
                .color(if step <= current { Color::primary() } else { Color::light_gray() })
                .overlay(
                    text!("{}", step + 1)
                        .color(if step <= current { Color::white() } else { Color::gray() })
                )
        })
    )
    .spacing(20.0)
}

fn personal_info_step(form_data: Binding<CompleteFormData>) -> impl View {
    vstack((
        text("Personal Information").font_size(20.0),
        
        text_field(s!(form_data.personal.first_name.clone()))
            .label("First Name")
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.personal.first_name = value;
                        data
                    });
                }
            }),
            
        text_field(s!(form_data.personal.last_name.clone()))
            .label("Last Name")
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.personal.last_name = value;
                        data
                    });
                }
            }),
            
        date_picker(s!(form_data.personal.birth_date))
            .label("Date of Birth")
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.personal.birth_date = value;
                        data
                    });
                }
            }),
    ))
    .spacing(15.0)
}

fn contact_info_step(form_data: Binding<CompleteFormData>) -> impl View {
    vstack((
        text("Contact Information").font_size(20.0),
        
        text_field(s!(form_data.contact.email.clone()))
            .label("Email")
            .input_type(InputType::Email)
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.contact.email = value;
                        data
                    });
                }
            }),
            
        text_field(s!(form_data.contact.phone.clone()))
            .label("Phone")
            .input_type(InputType::Tel)
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.contact.phone = value;
                        data
                    });
                }
            }),
    ))
    .spacing(15.0)
}

fn preferences_step(form_data: Binding<CompleteFormData>) -> impl View {
    vstack((
        text("Preferences").font_size(20.0),
        
        toggle(s!(form_data.preferences.newsletter))
            .label("Subscribe to newsletter")
            .on_change({
                let form_data = form_data.clone();
                move |value| {
                    form_data.update(|data| {
                        data.preferences.newsletter = value;
                        data
                    });
                }
            }),
            
        picker(s!(form_data.preferences.theme.clone()), vec![
            ("light", "Light Theme"),
            ("dark", "Dark Theme"),
            ("auto", "Auto"),
        ))
        .label("Theme Preference")
        .on_change({
            let form_data = form_data.clone();
            move |value| {
                form_data.update(|data| {
                    data.preferences.theme = value;
                    data
                });
            }
        }),
    ))
    .spacing(15.0)
}

fn form_navigation(current_step: Binding<usize>, total_steps: usize) -> impl View {
    hstack((
        button("Back")
            .disabled(s!(current_step == 0))
            .action({
                let current_step = current_step.clone();
                move |_| current_step.update(|step| step.saturating_sub(1))
            }),
            
        spacer(),
        
        button(s!(if current_step == total_steps - 1 { "Submit" } else { "Next" }))
            .action({
                let current_step = current_step.clone();
                move |_| {
                    if current_step.get() < total_steps - 1 {
                        current_step.update(|step| step + 1);
                    } else {
                        println!("Form submitted!");
                    }
                }
            }),
    ))
}

#[derive(Clone, Default)]
struct CompleteFormData {
    personal: PersonalInfo,
    contact: ContactInfo,
    preferences: Preferences,
}

#[derive(Clone, Default)]
struct PersonalInfo {
    first_name: String,
    last_name: String,
    birth_date: Date,
}

#[derive(Clone, Default)]
struct ContactInfo {
    email: String,
    phone: String,
}

#[derive(Clone, Default)]
struct Preferences {
    newsletter: bool,
    theme: String,
}
```

## Form Accessibility

### Accessible Form Controls

```rust,ignore
fn accessible_form_demo() -> impl View {
    let name = binding(String::new());
    let email = binding(String::new());
    let is_valid = s!(validate_email(&email));
    
    form([
        // Proper labeling
        text_field(name.clone())
            .accessibility_label("Full name")
            .accessibility_hint("Enter your first and last name")
            .required(true),
            
        // Error states
        text_field(email.clone())
            .accessibility_label("Email address")
            .accessibility_invalid(!is_valid.get())
            .accessibility_error_message(s!(if is_valid { 
                None 
            } else { 
                Some("Please enter a valid email address".to_string()) 
            }))
            .required(true),
            
        s!(if !is_valid {
            Some(
                text("Please enter a valid email address")
                    .color(Color::red())
                    .accessibility_role(AccessibilityRole::Alert)
                    .accessibility_live_region(LiveRegion::Polite)
            )
        } else {
            None
        }),
        
        // Fieldsets for grouped controls
        fieldset([
            legend("Contact Preferences"),
            
            checkbox(binding(true))
                .label("Email updates")
                .accessibility_hint("Receive updates via email"),
                
            checkbox(binding(false))
                .label("SMS updates")
                .accessibility_hint("Receive updates via text message"),
        )),
        
        button("Submit")
            .accessibility_hint("Submit the form after filling all required fields")
            .disabled(s!(name.is_empty() || !is_valid)),
    ))
    .spacing(15.0)
}
```

## Testing Form Components

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_form_validation() {
        let form = UserRegistrationForm {
            first_name: "".to_string(),
            last_name: "Smith".to_string(),
            email: "invalid-email".to_string(),
        };
        
        let errors = validate_registration_form(&form);
        assert_eq!(errors.len(), 2);
        assert!(errors.contains(&"First name is required".to_string()));
        assert!(errors.contains(&"Please enter a valid email address".to_string()));
        
        let valid_form = UserRegistrationForm {
            first_name: "John".to_string(),
            last_name: "Smith".to_string(),
            email: "john@example.com".to_string(),
        };
        
        let errors = validate_registration_form(&valid_form);
        assert!(errors.is_empty());
    }
    
    #[test]
    fn test_email_validation() {
        assert!(!validate_email(""));
        assert!(!validate_email("invalid"));
        assert!(!validate_email("@domain.com"));
        assert!(validate_email("user@domain.com"));
        assert!(validate_email("test.email@example.co.uk"));
    }
    
    #[test]
    fn test_multi_step_navigation() {
        let current_step = binding(0);
        
        // Test forward navigation
        current_step.update(|step| (step + 1).min(2));
        assert_eq!(current_step.get(), 1);
        
        current_step.update(|step| (step + 1).min(2));
        assert_eq!(current_step.get(), 2);
        
        // Test boundary
        current_step.update(|step| (step + 1).min(2));
        assert_eq!(current_step.get(), 2);
        
        // Test backward navigation
        current_step.update(|step| step.saturating_sub(1));
        assert_eq!(current_step.get(), 1);
        
        current_step.update(|step| step.saturating_sub(1));
        assert_eq!(current_step.get(), 0);
    }
}
```

## Summary

WaterUI's form system provides:

- **Comprehensive Controls**: Text, number, date, file, and selection inputs
- **Validation**: Field-level and form-level validation with error messages
- **Complex Layouts**: Multi-step forms, fieldsets, and responsive layouts
- **Accessibility**: Proper labeling, error states, and screen reader support
- **Reactive State**: Automatic UI updates with reactive bindings
- **Performance**: Efficient validation and state management

Key best practices:
- Use proper form structure with labels and validation
- Implement progressive validation with clear error messages
- Consider accessibility from the beginning
- Test form validation logic thoroughly
- Use appropriate input types for better UX
- Provide clear feedback during form submission

Next: [Navigation Components](12-navigation.md)