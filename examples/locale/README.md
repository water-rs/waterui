# Locale Example

This example demonstrates WaterUI's internationalization (i18n) capabilities.

## Features

- **Locale-aware date formatting** - Dates display according to regional conventions
- **CLDR plural rules** - Proper pluralization for all languages (e.g., Russian has one/few/many)
- **Unit formatting** - Localized unit symbols (米/メートル/m)
- **Number formatting** - Regional number separators

## Supported Locales

| Code | Language |
|------|----------|
| en | English |
| zh-CN | Chinese (Simplified) |
| ja | Japanese |
| ko | Korean |
| de | German |
| fr | French |
| es | Spanish |
| ru | Russian |

## Running

```bash
water run --platform ios
# or
water run --platform android
```

## Native Integration

Native backends can inject the system locale using FFI:

```c
waterui_env_install_locale_tag(env, "zh-Hans-CN");
```

## Example Output

**English:**
- Date: March 20, 2006
- Plural: "I have 5 apples"
- Length: "1500m"

**German:**
- Date: 20. März 2006
- Plural: "Ich habe 5 Äpfel"
- Length: "1500m"

**Japanese:**
- Date: 2006年3月20日
- Plural: "私はリンゴを5個持っています"
- Length: "1500メートル"
