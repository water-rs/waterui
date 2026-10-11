// Twin of examples/locale: world-fair kiosk in its initial en-US state.
//
// The example drives localization through WaterUI's i18n tables; the twin
// hardcodes the en-US strings from i18n/en-US.toml and formats the date/unit
// values through Foundation, which shares CLDR rules with WaterUI's locale
// layer. Detected-locale and timezone lines read the host environment at
// runtime, matching the example's system-derived values.

import SwiftUI

struct LocaleTwin: View {
  @State private var selection = "en-US"

  private static let localeOptions: [(String, String)] = [
    ("English (US)", "en-US"),
    ("English (UK)", "en-GB"),
    ("中文 (简体)", "zh"),
    ("中文 (台灣)", "zh-TW"),
    ("中文 (香港)", "zh-HK"),
    ("日本語", "ja"),
    ("한국어", "ko"),
    ("Deutsch", "de"),
    ("Français", "fr"),
    ("Español", "es"),
    ("Русский", "ru"),
  ]

  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI World Fair").font(.system(size: 28)).fontWeight(.bold)
        Text("Live translations for a tiny world-fair kiosk").font(.system(size: 14))
        wuiDivider()
        languageBooth
        wuiDivider()
        localizedContent
        wuiDivider()
        formattedContent
      }
      .padding(16)
    }
  }

  private var detectedLocale: String {
    Locale.current.identifier.replacingOccurrences(of: "_", with: "-")
  }

  private var languageBooth: some View {
    VStack(spacing: 10) {
      Text("Language Booth").font(.system(size: 16)).fontWeight(.bold)
      HStack {
        Text("Detected Locale:")
        Spacer(minLength: 0)
        Text(detectedLocale)
      }
      HStack {
        Text("Chosen Language:")
        Spacer(minLength: 0)
        Picker("", selection: $selection) {
          ForEach(Self.localeOptions, id: \.1) { name, code in
            Text(name).tag(code)
          }
        }
        .labelsHidden()
      }
    }
  }

  private var localizedContent: some View {
    VStack(spacing: 10) {
      VStack(spacing: 10) {
        Text("Welcome Desk").font(.system(size: 16)).fontWeight(.bold)
        Text("Welcome to the World Fair!").font(.system(size: 24))
      }
      wuiDivider()
      VStack(spacing: 10) {
        Text("Human Rights - Article 1").font(.system(size: 16)).fontWeight(.bold)
        Text(
          "All human beings are born free and equal in dignity and rights. They are endowed with reason and conscience and should act towards one another in a spirit of brotherhood."
        ).font(.system(size: 14))
      }
      wuiDivider()
      VStack(spacing: 10) {
        Text("Local Flavor").font(.system(size: 16)).fontWeight(.bold)
        HStack { Spacer(minLength: 0); Text("Color"); Spacer(minLength: 0) }
        HStack { Spacer(minLength: 0); Text("Favorite"); Spacer(minLength: 0) }
      }
      wuiDivider()
      VStack(spacing: 10) {
        Text("Passport Stamps").font(.system(size: 16)).fontWeight(.bold)
        Text("I have 0 passport stamps")
        Text("I have 1 passport stamp")
        Text("I have 2 passport stamps")
        Text("I have 5 passport stamps")
      }
    }
  }

  private var festivalDate: Date {
    var c = DateComponents()
    c.year = 2006; c.month = 3; c.day = 20
    c.calendar = Calendar(identifier: .gregorian)
    return c.date ?? Date()
  }

  private var formattedContent: some View {
    VStack(spacing: 10) {
      VStack(spacing: 10) {
        Text("Festival Date (2006-03-20)").font(.system(size: 16)).fontWeight(.bold)
        HStack {
          Text("Short:")
          Spacer(minLength: 0)
          Text(festivalDateString(.short))
        }
        HStack {
          Text("Long:")
          Spacer(minLength: 0)
          Text(festivalDateString(.long))
        }
        HStack {
          Text("Timezone:")
          Spacer(minLength: 0)
          Text(TimeZone.current.identifier)
        }
        HStack {
          Text("Kickoff (TZ):")
          Spacer(minLength: 0)
          Text(kickoffString)
        }
      }
      wuiDivider()
      VStack(spacing: 10) {
        Text("Distance Guide").font(.system(size: 16)).fontWeight(.bold)
        HStack {
          Text("City Walk:")
          Spacer(minLength: 0)
          Text(
            Measurement(value: 1500, unit: UnitLength.meters)
              .formatted(.measurement(width: .abbreviated, usage: .road))
          )
        }
        HStack {
          Text("Marathon Route:")
          Spacer(minLength: 0)
          Text(
            Measurement(value: 42.195, unit: UnitLength.kilometers)
              .formatted(.measurement(width: .abbreviated, usage: .road))
          )
        }
      }
    }
  }

  private func festivalDateString(_ style: DateFormatter.Style) -> String {
    let f = DateFormatter()
    f.dateStyle = style
    f.timeStyle = .none
    f.locale = Locale(identifier: "en-US")
    return f.string(from: festivalDate)
  }

  private var kickoffString: String {
    var c = DateComponents()
    c.year = 2006; c.month = 3; c.day = 20
    c.hour = 9; c.minute = 30; c.second = 0
    c.calendar = Calendar(identifier: .gregorian)
    let d = c.date ?? Date()
    let f = DateFormatter()
    f.dateStyle = .long
    f.timeStyle = .long
    f.locale = Locale(identifier: "en-US")
    return f.string(from: d)
  }
}
