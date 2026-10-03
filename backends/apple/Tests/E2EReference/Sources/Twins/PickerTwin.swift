// Twin of examples/picker: picker styles, date pickers, calendar,
// multi-date picker, color pickers, and the file-picker row in a padded
// scroll stack.
//
// WaterUI's Calendar month grid maps to SwiftUI's graphical DatePicker on
// macOS. The file picker has no SwiftUI primitive; the twin shows the same
// label row and the "No files selected" placeholder text.

import SwiftUI

private enum Fruit: String, CaseIterable, Identifiable {
  case apple = "Apple", banana = "Banana", cherry = "Cherry", date = "Date", elderberry = "Elderberry"
  var id: String { rawValue }
}

struct PickerTwin: View {
  @State private var automatic: Fruit = .apple
  @State private var menu: Fruit = .banana
  @State private var radio: Fruit = .cherry
  @State private var date = Self.day(2025, 1, 1)
  @State private var timeOnly = Self.dayTime(2025, 1, 1, 14, 30)
  @State private var datetime = Self.dayTime(2025, 6, 15, 9, 45)
  @State private var calendarDate = Self.day(2025, 6, 15)
  @State private var multiDates: Set<DateComponents> = []
  @State private var basicColor = Color(.sRGB, red: 0x33 / 255, green: 0x80 / 255, blue: 0xCC / 255)
  @State private var alphaColor = Color(.sRGB, red: 0xFF / 255, green: 0x4D / 255, blue: 0x80 / 255).opacity(0.8)
  @State private var hdrColor = Color(.sRGB, red: 0xE6 / 255, green: 0x1A / 255, blue: 0x66 / 255)

  private static func day(_ y: Int, _ m: Int, _ d: Int) -> Date {
    var c = DateComponents(); c.year = y; c.month = m; c.day = d
    c.calendar = Calendar(identifier: .gregorian)
    return c.date ?? Date()
  }

  private static func dayTime(_ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int) -> Date {
    var c = DateComponents(); c.year = y; c.month = m; c.day = d; c.hour = h; c.minute = min
    c.calendar = Calendar(identifier: .gregorian)
    return c.date ?? Date()
  }

  private var dateRange: ClosedRange<Date> { Self.day(2025, 1, 1) ... Self.day(2025, 12, 31) }

  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        VStack(spacing: 10) {
          Text("Picker Gallery").font(.title)
          Text("Demonstrating WaterUI form and picker components").font(.body)
        }
        wuiDivider()
        pickerStyles
        wuiDivider()
        datePickers
        wuiDivider()
        calendarSection
        wuiDivider()
        multiDateSection
        wuiDivider()
        colorSection
        wuiDivider()
        fileSection
        VStack(spacing: 10) {
          wuiDivider()
          Text("Built with WaterUI Picker Components").font(.caption)
        }
      }
      .padding(16)
    }
  }

  private func selectionLabel(_ fruit: Fruit) -> some View {
    // `hstack(("Selected: ", text!("…")))` — the space is inside the first
    // text, before the stack's spacing.
    HStack(spacing: 10) { Text("Selected: "); Text(fruit.rawValue) }
  }

  private var pickerStyles: some View {
    VStack(spacing: 10) {
      Text("Picker Styles").font(.headline)
      Text("Choose from different picker presentation styles").font(.body)
      Spacer(minLength: 0)
      Text("Automatic (default)").fontWeight(.bold)
      Picker("Automatic (default)", selection: $automatic) {
        ForEach(Fruit.allCases) { Text($0.rawValue).tag($0) }
      }
      selectionLabel(automatic)
      Spacer(minLength: 0)
      Text("Menu Style").fontWeight(.bold)
      Picker("Menu Style", selection: $menu) {
        ForEach(Fruit.allCases) { Text($0.rawValue).tag($0) }
      }
      .pickerStyle(.menu)
      selectionLabel(menu)
      Spacer(minLength: 0)
      Text("Radio Style").fontWeight(.bold)
      Picker("Radio Style", selection: $radio) {
        ForEach(Fruit.allCases) { Text($0.rawValue).tag($0) }
      }
      // iOS has no radio-group picker; .inline is its one-choice-visible list.
      #if os(iOS)
        .pickerStyle(.inline)
      #else
        .pickerStyle(.radioGroup)
      #endif
      selectionLabel(radio)
    }
    .padding(12)
  }

  private var datePickers: some View {
    VStack(spacing: 10) {
      Text("DatePicker").font(.headline)
      Text("Select dates and times with platform-native pickers").font(.body)
      Spacer(minLength: 0)
      DatePicker("Date Only", selection: $date, in: dateRange, displayedComponents: .date)
      Text("Selected date: \(date, format: .dateTime.year().month().day())")
      Spacer(minLength: 0)
      DatePicker("Time Only", selection: $timeOnly, displayedComponents: .hourAndMinute)
      Text("Selected time: \(timeOnly, format: .dateTime.hour().minute().second())")
      Spacer(minLength: 0)
      DatePicker("Date & Time", selection: $datetime, displayedComponents: [.date, .hourAndMinute])
      Text("Selected datetime: \(datetime, format: .dateTime.year().month().day().hour().minute().second())")
    }
    .padding(12)
  }

  private var calendarSection: some View {
    VStack(spacing: 10) {
      Text("Calendar").font(.headline)
      Text("Month-grid calendar with single-date selection and passive decorations").font(.body)
      Spacer(minLength: 0)
      DatePicker(
        "Trip Date", selection: $calendarDate, in: dateRange, displayedComponents: .date
      )
      .datePickerStyle(.graphical)
      Text("Selected calendar date: \(calendarDate, format: .dateTime.year().month().day())")
    }
    .padding(12)
  }

  private var multiDateSection: some View {
    VStack(spacing: 10) {
      Text("Multi-Date Picker").font(.headline)
      Text("Month-grid calendar for selecting multiple dates").font(.body)
      Spacer(minLength: 0)
      #if os(iOS)
        MultiDatePicker("Available Dates", selection: $multiDates, in: Self.day(2025, 1, 1) ..< Self.day(2026, 1, 1))
      #else
        // macOS has no MultiDatePicker; WUI's macOS realization is an
        // NSDatePicker + selection list, so the graphical DatePicker is the
        // closest visual match.
        DatePicker("Available Dates", selection: $calendarDate, in: dateRange, displayedComponents: .date)
          .datePickerStyle(.graphical)
      #endif
      Text("Selected dates: \(multiDates.count)")
    }
    .padding(12)
  }

  private func swatch(_ color: Color, _ label: String) -> some View {
    HStack(spacing: 10) {
      Text(label).fontWeight(.bold)
      // `text(": ")` — the example's space is inside the text.
      Text(": ")
      RoundedRectangle(cornerRadius: 0.1 * 32)
        .fill(color)
        .frame(width: 64, height: 32)
    }
  }

  private var colorSection: some View {
    VStack(spacing: 10) {
      Text("ColorPicker").font(.headline)
      Text("Select colors with optional alpha and HDR support").font(.body)
      Spacer(minLength: 0)
      ColorPicker("Basic Color", selection: $basicColor)
      swatch(basicColor, "Basic")
      Spacer(minLength: 0)
      ColorPicker("With Alpha", selection: $alphaColor, supportsOpacity: true)
      swatch(alphaColor, "Alpha")
      Spacer(minLength: 0)
      ColorPicker("HDR Color", selection: $hdrColor, supportsOpacity: false)
      swatch(hdrColor, "HDR")
    }
    .padding(12)
  }

  private var fileSection: some View {
    VStack(spacing: 10) {
      Text("FilePicker").font(.headline)
      Text("Select files from the device").font(.body)
      Spacer(minLength: 0)
      Button("Select Files") {}
      Spacer(minLength: 0)
      Text("Selected files:").fontWeight(.bold)
      Text("No files selected")
    }
    .padding(12)
  }
}
