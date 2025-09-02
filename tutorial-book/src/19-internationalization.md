# Internationalization

Building applications for global audiences requires comprehensive internationalization (i18n) support. WaterUI provides powerful tools for multi-language applications, localization, and cultural adaptations.

## Basic Internationalization

### Setting Up i18n

```rust,ignore
use waterui::*;
use nami::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
struct Locale {
    code: String,
    name: String,
    rtl: bool,
}

#[derive(Clone)]
struct I18nContext {
    current_locale: Binding<Locale>,
    translations: HashMap<String, HashMap<String, String>>,
    fallback_locale: String,
}

impl I18nContext {
    fn new() -> Self {
        let mut translations = HashMap::new();
        
        // English translations
        let mut en = HashMap::new();
        en.insert("app.title".to_string(), "WaterUI Demo App".to_string());
        en.insert("nav.home".to_string(), "Home".to_string());
        en.insert("nav.about".to_string(), "About".to_string());
        en.insert("nav.contact".to_string(), "Contact".to_string());
        en.insert("button.save".to_string(), "Save".to_string());
        en.insert("button.cancel".to_string(), "Cancel".to_string());
        en.insert("form.name".to_string(), "Name".to_string());
        en.insert("form.email".to_string(), "Email".to_string());
        en.insert("message.welcome".to_string(), "Welcome, {name}!".to_string());
        en.insert("date.format".to_string(), "%B %d, %Y".to_string());
        translations.insert("en".to_string(), en);
        
        // Spanish translations
        let mut es = HashMap::new();
        es.insert("app.title".to_string(), "Aplicación Demo WaterUI".to_string());
        es.insert("nav.home".to_string(), "Inicio".to_string());
        es.insert("nav.about".to_string(), "Acerca de".to_string());
        es.insert("nav.contact".to_string(), "Contacto".to_string());
        es.insert("button.save".to_string(), "Guardar".to_string());
        es.insert("button.cancel".to_string(), "Cancelar".to_string());
        es.insert("form.name".to_string(), "Nombre".to_string());
        es.insert("form.email".to_string(), "Correo electrónico".to_string());
        es.insert("message.welcome".to_string(), "¡Bienvenido, {name}!".to_string());
        es.insert("date.format".to_string(), "%d de %B de %Y".to_string());
        translations.insert("es".to_string(), es);
        
        // Arabic translations
        let mut ar = HashMap::new();
        ar.insert("app.title".to_string(), "تطبيق WaterUI التجريبي".to_string());
        ar.insert("nav.home".to_string(), "الرئيسية".to_string());
        ar.insert("nav.about".to_string(), "حول".to_string());
        ar.insert("nav.contact".to_string(), "اتصل بنا".to_string());
        ar.insert("button.save".to_string(), "حفظ".to_string());
        ar.insert("button.cancel".to_string(), "إلغاء".to_string());
        ar.insert("form.name".to_string(), "الاسم".to_string());
        ar.insert("form.email".to_string(), "البريد الإلكتروني".to_string());
        ar.insert("message.welcome".to_string(), "مرحباً، {name}!".to_string());
        ar.insert("date.format".to_string(), "%d %B %Y".to_string());
        translations.insert("ar".to_string(), ar);
        
        // Japanese translations
        let mut ja = HashMap::new();
        ja.insert("app.title".to_string(), "WaterUIデモアプリ".to_string());
        ja.insert("nav.home".to_string(), "ホーム".to_string());
        ja.insert("nav.about".to_string(), "概要".to_string());
        ja.insert("nav.contact".to_string(), "お問い合わせ".to_string());
        ja.insert("button.save".to_string(), "保存".to_string());
        ja.insert("button.cancel".to_string(), "キャンセル".to_string());
        ja.insert("form.name".to_string(), "名前".to_string());
        ja.insert("form.email".to_string(), "メールアドレス".to_string());
        ja.insert("message.welcome".to_string(), "ようこそ、{name}さん！".to_string());
        ja.insert("date.format".to_string(), "%Y年%m月%d日".to_string());
        translations.insert("ja".to_string(), ja);
        
        Self {
            current_locale: binding(Locale {
                code: "en".to_string(),
                name: "English".to_string(),
                rtl: false,
            }),
            translations,
            fallback_locale: "en".to_string(),
        }
    }
    
    fn t(&self, key: &str) -> String {
        self.t_with_params(key, &HashMap::new())
    }
    
    fn t_with_params(&self, key: &str, params: &HashMap<String, String>) -> String {
        let locale_code = &self.current_locale.get().code;
        
        let translation = self.translations
            .get(locale_code)
            .and_then(|locale_translations| locale_translations.get(key))
            .or_else(|| {
                self.translations
                    .get(&self.fallback_locale)
                    .and_then(|fallback_translations| fallback_translations.get(key))
            })
            .cloned()
            .unwrap_or_else(|| key.to_string());
            
        // Simple parameter substitution
        let mut result = translation;
        for (param_key, param_value) in params {
            result = result.replace(&format!("{{{}}}", param_key), param_value);
        }
        
        result
    }
    
    fn available_locales(&self) -> Vec<Locale> {
        vec![
            Locale { code: "en".to_string(), name: "English".to_string(), rtl: false },
            Locale { code: "es".to_string(), name: "Español".to_string(), rtl: false },
            Locale { code: "ar".to_string(), name: "العربية".to_string(), rtl: true },
            Locale { code: "ja".to_string(), name: "日本語".to_string(), rtl: false },
        ]
    }
}

fn i18n_demo() -> impl View {
    let i18n = I18nContext::new();
    let user_name = binding("Alice".to_string());
    
    i18n_provider(i18n.clone(),
        vstack((
            // App header with localized title
            app_header(&i18n),
            
            // Language selector
            language_selector(&i18n),
            
            // Localized navigation
            localized_navigation(&i18n),
            
            // Localized form
            localized_form(&i18n, user_name.clone()),
            
            // Date and time localization
            date_time_localization(&i18n),
            
            // Number and currency formatting
            number_formatting(&i18n),
        ))
        .spacing(20.0)
        .padding(20.0)
        .text_direction(s!(if i18n.current_locale.get().rtl { 
            TextDirection::RTL 
        } else { 
            TextDirection::LTR 
        }))
    )
}

fn app_header(i18n: &I18nContext) -> impl View {
    text!(i18n.t("app.title"))
        .font_size(24.0)
        .font_weight(FontWeight::Bold)
        .accessibility_role(AccessibilityRole::Heading)
        .accessibility_level(1)
}

fn language_selector(i18n: &I18nContext) -> impl View {
    let current_locale = i18n.current_locale.clone();
    let available_locales = i18n.available_locales();
    
    hstack((
        text("Language:"),
        
        picker(
            s!(current_locale.code.clone()),
            available_locales.into_iter().map(|locale| {
                (locale.code.clone(), locale.name.as_str())
            }).collect()
        )
        .on_change({
            let i18n = i18n.clone();
            let current_locale = current_locale.clone();
            move |locale_code| {
                if let Some(locale) = i18n.available_locales().into_iter()
                    .find(|l| l.code == locale_code) {
                    current_locale.set(locale);
                }
            }
        }),
    ))
    .spacing(10.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn localized_navigation(i18n: &I18nContext) -> impl View {
    hstack((
        nav_button(&i18n.t("nav.home"), "/"),
        nav_button(&i18n.t("nav.about"), "/about"),
        nav_button(&i18n.t("nav.contact"), "/contact"),
    ))
    .spacing(15.0)
}

fn nav_button(label: &str, href: &str) -> impl View {
    button(label)
        .accessibility_hint(&format!("Navigate to {}", label))
        .action(move |_| {
            println!("Navigate to: {}", href);
        })
}

fn localized_form(i18n: &I18nContext, user_name: Binding<String>) -> impl View {
    let email = binding(String::new());
    
    vstack((
        text_field(user_name.clone())
            .label(&i18n.t("form.name"))
            .placeholder(&i18n.t("form.name")),
            
        text_field(email.clone())
            .label(&i18n.t("form.email"))
            .placeholder(&i18n.t("form.email")),
            
        hstack((
            button(&i18n.t("button.save"))
                .action(move |_| {
                    println!("Save clicked");
                }),
                
            button(&i18n.t("button.cancel"))
                .style(ButtonStyle::Secondary)
                .action(move |_| {
                    println!("Cancel clicked");
                }),
        ))
        .spacing(10.0),
        
        // Localized welcome message with parameter
        text!(i18n.t_with_params("message.welcome", &{
            let mut params = HashMap::new();
            params.insert("name".to_string(), user_name.get());
            params
        }))
        .color(Color::primary()),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}
```

## Date and Time Localization

```rust,ignore
fn date_time_localization(i18n: &I18nContext) -> impl View {
    let current_time = chrono::Local::now();
    let current_locale = i18n.current_locale.get();
    
    vstack((
        text("Date and Time Localization").font_size(16.0),
        
        // Formatted dates
        localized_date_examples(current_time, &current_locale),
        
        // Time zones
        timezone_examples(current_time),
        
        // Relative time
        relative_time_examples(&current_locale),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn localized_date_examples(
    time: chrono::DateTime<chrono::Local>,
    locale: &Locale
) -> impl View {
    vstack((
        text("Date Formats:").font_weight(FontWeight::Medium),
        
        // Different date formats based on locale
        text!(format_date_for_locale(time, locale, DateFormat::Short)),
        text!(format_date_for_locale(time, locale, DateFormat::Medium)),
        text!(format_date_for_locale(time, locale, DateFormat::Long)),
        text!(format_date_for_locale(time, locale, DateFormat::Full)),
    ))
    .spacing(5.0)
}

#[derive(Clone)]
enum DateFormat {
    Short,
    Medium,
    Long,
    Full,
}

fn format_date_for_locale(
    time: chrono::DateTime<chrono::Local>,
    locale: &Locale,
    format: DateFormat
) -> String {
    match locale.code.as_str() {
        "en" => match format {
            DateFormat::Short => time.format("%m/%d/%Y").to_string(),
            DateFormat::Medium => time.format("%b %d, %Y").to_string(),
            DateFormat::Long => time.format("%B %d, %Y").to_string(),
            DateFormat::Full => time.format("%A, %B %d, %Y").to_string(),
        },
        "es" => match format {
            DateFormat::Short => time.format("%d/%m/%Y").to_string(),
            DateFormat::Medium => time.format("%d %b %Y").to_string(),
            DateFormat::Long => time.format("%d de %B de %Y").to_string(),
            DateFormat::Full => time.format("%A, %d de %B de %Y").to_string(),
        },
        "ja" => match format {
            DateFormat::Short => time.format("%Y/%m/%d").to_string(),
            DateFormat::Medium => time.format("%Y年%m月%d日").to_string(),
            DateFormat::Long => time.format("%Y年%m月%d日").to_string(),
            DateFormat::Full => time.format("%Y年%m月%d日(%a)").to_string(),
        },
        "ar" => match format {
            DateFormat::Short => time.format("%d/%m/%Y").to_string(),
            DateFormat::Medium => time.format("%d %b %Y").to_string(),
            DateFormat::Long => time.format("%d %B %Y").to_string(),
            DateFormat::Full => time.format("%A، %d %B %Y").to_string(),
        },
        _ => time.format("%Y-%m-%d").to_string(),
    }
}

fn timezone_examples(time: chrono::DateTime<chrono::Local>) -> impl View {
    vstack((
        text("Time Zones:").font_weight(FontWeight::Medium),
        
        text!("Local: {}", time.format("%H:%M:%S %Z")),
        text!("UTC: {}", time.with_timezone(&chrono::Utc).format("%H:%M:%S UTC")),
        text!("Tokyo: {}", time.with_timezone(&chrono_tz::Asia::Tokyo).format("%H:%M:%S JST")),
        text!("London: {}", time.with_timezone(&chrono_tz::Europe::London).format("%H:%M:%S %Z")),
    ))
    .spacing(5.0)
}

fn relative_time_examples(locale: &Locale) -> impl View {
    let now = chrono::Utc::now();
    let hour_ago = now - chrono::Duration::hours(1);
    let day_ago = now - chrono::Duration::days(1);
    let week_ago = now - chrono::Duration::weeks(1);
    
    vstack((
        text("Relative Time:").font_weight(FontWeight::Medium),
        
        text!(relative_time_string(hour_ago, now, locale)),
        text!(relative_time_string(day_ago, now, locale)),
        text!(relative_time_string(week_ago, now, locale)),
    ))
    .spacing(5.0)
}

fn relative_time_string(
    time: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
    locale: &Locale
) -> String {
    let duration = now.signed_duration_since(time);
    
    match locale.code.as_str() {
        "en" => {
            if duration.num_hours() < 1 {
                format!("{} minutes ago", duration.num_minutes())
            } else if duration.num_days() < 1 {
                format!("{} hours ago", duration.num_hours())
            } else if duration.num_weeks() < 1 {
                format!("{} days ago", duration.num_days())
            } else {
                format!("{} weeks ago", duration.num_weeks())
            }
        },
        "es" => {
            if duration.num_hours() < 1 {
                format!("hace {} minutos", duration.num_minutes())
            } else if duration.num_days() < 1 {
                format!("hace {} horas", duration.num_hours())
            } else if duration.num_weeks() < 1 {
                format!("hace {} días", duration.num_days())
            } else {
                format!("hace {} semanas", duration.num_weeks())
            }
        },
        "ja" => {
            if duration.num_hours() < 1 {
                format!("{}分前", duration.num_minutes())
            } else if duration.num_days() < 1 {
                format!("{}時間前", duration.num_hours())
            } else if duration.num_weeks() < 1 {
                format!("{}日前", duration.num_days())
            } else {
                format!("{}週間前", duration.num_weeks())
            }
        },
        "ar" => {
            if duration.num_hours() < 1 {
                format!("منذ {} دقيقة", duration.num_minutes())
            } else if duration.num_days() < 1 {
                format!("منذ {} ساعة", duration.num_hours())
            } else if duration.num_weeks() < 1 {
                format!("منذ {} يوم", duration.num_days())
            } else {
                format!("منذ {} أسبوع", duration.num_weeks())
            }
        },
        _ => format!("{} ago", duration),
    }
}
```

## Number and Currency Formatting

```rust,ignore
fn number_formatting(i18n: &I18nContext) -> impl View {
    let current_locale = i18n.current_locale.get();
    let sample_number = 1234567.89;
    let sample_currency = 99.95;
    
    vstack((
        text("Number and Currency Formatting").font_size(16.0),
        
        number_format_examples(sample_number, &current_locale),
        currency_format_examples(sample_currency, &current_locale),
        percentage_format_examples(&current_locale),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn number_format_examples(number: f64, locale: &Locale) -> impl View {
    vstack((
        text("Number Formats:").font_weight(FontWeight::Medium),
        
        text!(format_number_for_locale(number, locale)),
        text!(format_number_with_precision(number, locale, 0)),
        text!(format_number_with_precision(number, locale, 2)),
        text!(format_large_number(number * 1000.0, locale)),
    ))
    .spacing(5.0)
}

fn format_number_for_locale(number: f64, locale: &Locale) -> String {
    match locale.code.as_str() {
        "en" => format!("{:,.2}", number), // 1,234,567.89
        "es" => format!("{:.2}", number).replace('.', ",").replace(',', "."), // 1.234.567,89
        "fr" => format!("{:.2}", number).replace('.', ","), // 1 234 567,89 (simplified)
        "de" => format!("{:.2}", number).replace('.', ","), // 1.234.567,89
        _ => format!("{:.2}", number),
    }
}

fn format_number_with_precision(number: f64, locale: &Locale, precision: usize) -> String {
    match locale.code.as_str() {
        "en" => format!("{:.*}", precision, number),
        "es" => {
            let formatted = format!("{:.*}", precision, number);
            if precision > 0 {
                formatted.replace('.', ",")
            } else {
                formatted
            }
        },
        _ => format!("{:.*}", precision, number),
    }
}

fn format_large_number(number: f64, locale: &Locale) -> String {
    match locale.code.as_str() {
        "en" => {
            if number >= 1_000_000_000.0 {
                format!("{:.1}B", number / 1_000_000_000.0)
            } else if number >= 1_000_000.0 {
                format!("{:.1}M", number / 1_000_000.0)
            } else if number >= 1_000.0 {
                format!("{:.1}K", number / 1_000.0)
            } else {
                format!("{:.0}", number)
            }
        },
        "es" => {
            if number >= 1_000_000_000.0 {
                format!("{:.1}mil M", number / 1_000_000_000.0)
            } else if number >= 1_000_000.0 {
                format!("{:.1}M", number / 1_000_000.0)
            } else if number >= 1_000.0 {
                format!("{:.1}mil", number / 1_000.0)
            } else {
                format!("{:.0}", number)
            }
        },
        _ => format!("{:.2}", number),
    }
}

fn currency_format_examples(amount: f64, locale: &Locale) -> impl View {
    vstack((
        text("Currency Formats:").font_weight(FontWeight::Medium),
        
        text!(format_currency(amount, locale, "USD")),
        text!(format_currency(amount, locale, "EUR")),
        text!(format_currency(amount, locale, "JPY")),
        text!(format_currency(amount, locale, "GBP")),
    ))
    .spacing(5.0)
}

fn format_currency(amount: f64, locale: &Locale, currency: &str) -> String {
    let symbol = match currency {
        "USD" => "$",
        "EUR" => "€", 
        "JPY" => "¥",
        "GBP" => "£",
        _ => currency,
    };
    
    let formatted_amount = match currency {
        "JPY" => format!("{:.0}", amount * 100.0), // No decimals for JPY
        _ => format!("{:.2}", amount),
    };
    
    match locale.code.as_str() {
        "en" => format!("{}{}", symbol, formatted_amount),
        "es" => format!("{} {}", formatted_amount.replace('.', ","), symbol),
        "ja" => format!("{}{}", symbol, formatted_amount),
        "ar" => format!("{} {}", symbol, formatted_amount),
        _ => format!("{}{}", symbol, formatted_amount),
    }
}

fn percentage_format_examples(locale: &Locale) -> impl View {
    let percentages = vec![0.1, 0.25, 0.5, 0.75, 1.0];
    
    vstack((
        text("Percentage Formats:").font_weight(FontWeight::Medium),
        
        vstack(
            percentages.into_iter().map(|pct| {
                text!(format_percentage(pct, locale))
            })
        )
        .spacing(3.0),
    ))
    .spacing(5.0)
}

fn format_percentage(value: f64, locale: &Locale) -> String {
    let percentage = value * 100.0;
    
    match locale.code.as_str() {
        "en" => format!("{:.1}%", percentage),
        "es" => format!("{:.1} %", percentage.to_string().replace('.', ",")),
        "fr" => format!("{:.1} %", percentage.to_string().replace('.', ",")),
        _ => format!("{:.1}%", percentage),
    }
}
```

## RTL (Right-to-Left) Support

```rust,ignore
fn rtl_support_demo() -> impl View {
    let current_locale = binding(Locale {
        code: "ar".to_string(),
        name: "العربية".to_string(),
        rtl: true,
    });
    
    let is_rtl = s!(current_locale.rtl);
    
    vstack((
        text("RTL Support Demo")
            .font_size(20.0)
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(1),
            
        // Language toggle
        hstack((
            button("English (LTR)")
                .action({
                    let current_locale = current_locale.clone();
                    move |_| {
                        current_locale.set(Locale {
                            code: "en".to_string(),
                            name: "English".to_string(),
                            rtl: false,
                        });
                    }
                }),
                
            button("العربية (RTL)")
                .action({
                    let current_locale = current_locale.clone();
                    move |_| {
                        current_locale.set(Locale {
                            code: "ar".to_string(),
                            name: "العربية".to_string(),
                            rtl: true,
                        });
                    }
                }),
        ))
        .spacing(10.0),
        
        // RTL-aware layout
        rtl_aware_content(is_rtl.clone()),
        
        // Form with RTL support
        rtl_form_example(is_rtl.clone()),
        
        // Navigation with RTL
        rtl_navigation_example(is_rtl.clone()),
    ))
    .spacing(20.0)
    .padding(20.0)
    .text_direction(s!(if is_rtl { TextDirection::RTL } else { TextDirection::LTR }))
}

fn rtl_aware_content(is_rtl: Signal<bool>) -> impl View {
    vstack((
        text(s!(if is_rtl { 
            "محتوى يدعم الاتجاه من اليمين إلى اليسار" 
        } else { 
            "Content with RTL support" 
        }))
        .font_size(16.0),
        
        // Card with proper RTL layout
        hstack((
            // Avatar/icon (position changes based on direction)
            circle()
                .width(40.0)
                .height(40.0)
                .color(Color::primary()),
                
            vstack((
                text(s!(if is_rtl { 
                    "اسم المستخدم" 
                } else { 
                    "User Name" 
                }))
                .font_weight(FontWeight::Bold),
                text(s!(if is_rtl { 
                    "وصف قصير للمستخدم" 
                } else { 
                    "A short user description" 
                }))
                .color(Color::secondary()),
            ))
            .spacing(5.0)
            .flex(1),
            
            // Action buttons (also flip position)
            hstack((
                button("⋮"),
                button("→")
                    .rotation(s!(if is_rtl { 180.0 } else { 0.0 })), // Flip arrow
            ))
            .spacing(5.0),
        ))
        .spacing(s!(if is_rtl { 15.0 } else { 15.0 })) // Spacing remains the same
        .padding(15.0)
        .background(Color::surface())
        .corner_radius(8.0),
    ))
    .spacing(15.0)
}

fn rtl_form_example(is_rtl: Signal<bool>) -> impl View {
    let name = binding(String::new());
    let email = binding(String::new());
    
    vstack((
        text(s!(if is_rtl { 
            "نموذج مع دعم RTL" 
        } else { 
            "Form with RTL Support" 
        }))
        .font_size(16.0),
        
        text_field(name.clone())
            .label(s!(if is_rtl { "الاسم" } else { "Name" }))
            .placeholder(s!(if is_rtl { "أدخل اسمك" } else { "Enter your name" }))
            .text_direction(s!(if is_rtl { TextDirection::RTL } else { TextDirection::LTR })),
            
        text_field(email.clone())
            .label(s!(if is_rtl { "البريد الإلكتروني" } else { "Email" }))
            .placeholder(s!(if is_rtl { "أدخل بريدك الإلكتروني" } else { "Enter your email" }))
            .text_direction(TextDirection::LTR), // Email is always LTR
            
        hstack((
            // Button order changes for RTL
            s!(if is_rtl {
                // In RTL, Cancel comes first (right side)
                (
                    button("إلغاء")
                        .style(ButtonStyle::Secondary),
                    button("حفظ")
                        .style(ButtonStyle::Primary),
                )
            } else {
                // In LTR, Save comes last (right side)
                (
                    button("Cancel")
                        .style(ButtonStyle::Secondary),
                    button("Save")
                        .style(ButtonStyle::Primary),
                )
            }),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
    .padding(15.0)
    .background(Color::surface())
    .corner_radius(8.0)
}

fn rtl_navigation_example(is_rtl: Signal<bool>) -> impl View {
    hstack((
        // Navigation items
        nav_item(s!(if is_rtl { "الرئيسية" } else { "Home" }), true),
        nav_item(s!(if is_rtl { "المنتجات" } else { "Products" }), false),
        nav_item(s!(if is_rtl { "حول" } else { "About" }), false),
        nav_item(s!(if is_rtl { "اتصل بنا" } else { "Contact" }), false),
        
        spacer(),
        
        // User menu (position adapts to text direction)
        user_menu(is_rtl.clone()),
    ))
    .spacing(0.0) // Navigation items handle their own spacing
    .padding(10.0)
    .background(Color::primary())
}

fn nav_item(label: Signal<String>, active: bool) -> impl View {
    text!(label)
        .padding(EdgeInsets::symmetric(15.0, 10.0))
        .color(if active { Color::white() } else { Color::white().opacity(0.8) })
        .font_weight(if active { FontWeight::Bold } else { FontWeight::Normal })
        .background(if active { 
            Color::white().opacity(0.2) 
        } else { 
            Color::transparent() 
        })
        .corner_radius(6.0)
        .on_tap(move |_| {
            println!("Navigate to: {:?}", label);
        })
}

fn user_menu(is_rtl: Signal<bool>) -> impl View {
    let menu_open = binding(false);
    
    hstack((
        s!(if is_rtl {
            // In RTL, dropdown arrow comes first
            Some(
                text(s!(if menu_open { "▲" } else { "▼" }))
                    .color(Color::white())
            )
        } else {
            None
        }),
        
        circle()
            .width(32.0)
            .height(32.0)
            .color(Color::white())
            .overlay(
                text("U")
                    .color(Color::primary())
                    .font_weight(FontWeight::Bold)
            ),
            
        s!(if !is_rtl {
            // In LTR, dropdown arrow comes after
            Some(
                text(s!(if menu_open { "▲" } else { "▼" }))
                    .color(Color::white())
            )
        } else {
            None
        }),
    ))
    .spacing(8.0)
    .on_tap({
        let menu_open = menu_open.clone();
        move |_| {
            menu_open.update(|open| !open);
        }
    })
}
```

## Summary

WaterUI's internationalization features provide:

- **Multi-language Support**: Translation system with fallback handling
- **Date and Time Localization**: Culture-specific formatting and time zones
- **Number and Currency Formatting**: Locale-aware numeric displays
- **RTL Support**: Right-to-left layout with proper text direction
- **Cultural Adaptations**: Layout adjustments for different cultures
- **Dynamic Language Switching**: Runtime locale changes

Key best practices:
- Extract all user-facing strings into translation files
- Use parameter substitution for dynamic content
- Test with different locales and text directions
- Consider cultural differences in layout and design
- Implement proper fallback mechanisms
- Use Unicode-aware text processing
- Test with actual native speakers when possible

Next: [Building a Todo Application](20-todo-app.md) - This chapter already exists and is complete.

Next available: [Media Player](21-media-player.md)