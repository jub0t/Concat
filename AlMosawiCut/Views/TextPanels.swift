import SwiftUI
import UniformTypeIdentifiers

struct TextPanel: View {
    @Bindable var model: EditorModel
    @State private var tab: TextTab
    @FocusState private var focused: Bool
    @State private var library = FontLibrary.shared
    @State private var importingFont = false
    @State private var fontError: String?

    init(model: EditorModel, initialTab: TextTab) {
        self.model = model
        _tab = State(initialValue: initialTab)
    }

    var body: some View {
        if let i = model.selectedTextIndex {
            let item = model.project.texts[i]
            let tabs = item.shape == nil ? TextTab.allCases : [.style, .animate]
            VStack(alignment: .leading, spacing: 12) {
                Picker("", selection: $tab) {
                    ForEach(tabs, id: \.self) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                switch tabs.contains(tab) ? tab : tabs[0] {
                case .edit: editTab(item)
                case .style: styleTab(item)
                case .font: fontTab(item)
                case .animate: animateTab
                }
            }
        } else { NeedsSelection(text: "Select a text on the timeline.") }
    }

    private func editTab(_ item: TextOverlay) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField("Text", text: model.textBinding(\.text, fallback: ""), axis: .vertical)
                .lineLimit(1...4)
                .focused($focused)
                .padding(12)
                .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
            HStack {
                Chips(options: TextCase.allCases.map { ($0, $0.label) }, selection: model.textBinding(\.textCase, fallback: .none))
                Toggle("Bold", isOn: model.textBinding(\.bold, fallback: true)).font(.caption.weight(.semibold)).fixedSize()
            }
            LabeledSlider(title: "Size", value: model.textBinding(\.size, fallback: 0.07), range: 0.025...0.2, format: { "\(Int($0 * 1000))" }, reset: 0.07)
            LabeledSlider(title: "Starts at", value: model.textBinding(\.start, fallback: 0), range: 0...max(0.1, model.duration), format: { $0.timecode })
            LabeledSlider(title: "Duration", value: model.textBinding(\.duration, fallback: 3), range: 0.2...max(0.3, min(60, model.duration)),
                          format: { String(format: "%.1fs", $0) })
        }
    }

    private func styleTab(_ item: TextOverlay) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            if item.shape == nil {
                Chips(options: TextStyle.allCases.map { ($0, $0.label) }, selection: model.textBinding(\.style, fallback: .outline))
            } else {
                Chips(options: [(TextStyle.plain, "Filled"), (TextStyle.outline, "Outlined")], selection: model.textBinding(\.style, fallback: .plain))
                LabeledSlider(title: "Size", value: model.textBinding(\.size, fallback: 0.05), range: 0.01...0.25, format: { "\(Int($0 * 1000))" })
            }
            swatches(item.shape == nil ? "Text" : "Fill", model.textBinding(\.color, fallback: .white))
            swatches(accentLabel(item), model.textBinding(\.accent, fallback: .black))
            LabeledSlider(title: "Opacity", value: model.textBinding(\.opacity, fallback: 1), range: 0...1, format: { "\(Int($0 * 100))%" }, reset: 1)
            LabeledSlider(title: "Rotation", value: model.textBinding(\.rotation, fallback: 0), range: -180...180, format: { "\(Int($0))°" }, reset: 0)
            if item.isCaption {
                Button { model.applyStyleToAllCaptions(from: item) } label: {
                    Label("Use this style for every caption", systemImage: "captions.bubble.fill").font(.caption.weight(.bold))
                }
                .buttonStyle(.plain).foregroundStyle(Theme.accent)
            }
        }
    }

    private func accentLabel(_ item: TextOverlay) -> String {
        if item.shape != nil { return "Outline" }
        switch item.style {
        case .box: return "Box"
        case .outline: return "Outline"
        case .shadow: return "Shadow"
        case .glow: return "Glow"
        case .plain: return "Accent"
        }
    }

    private func fontTab(_ item: TextOverlay) -> some View {
        let fontsGrid = [GridItem(.adaptive(minimum: 96), spacing: 8)]
        let custom = model.textBinding(\.customFont, fallback: nil)
        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                SectionLabel("My fonts")
                Button { importingFont = true } label: {
                    Label("Add font", systemImage: "plus")
                        .font(.caption.weight(.bold))
                        .padding(.horizontal, 10).frame(height: 28)
                        .background(Theme.accent, in: Capsule())
                        .foregroundStyle(.black)
                }
                .buttonStyle(.plain)
            }
            if library.fonts.isEmpty {
                Text("Add .ttf, .otf or .ttc files from Files. They stay in the app for every project.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            } else {
                LazyVGrid(columns: fontsGrid, spacing: 8) {
                    ForEach(library.fonts) { entry in
                        fontTile(sample: Font(TextRasterizer.customFont(entry.postScriptName, size: 19, bold: false) ?? .systemFont(ofSize: 19)),
                                 name: entry.displayName, on: item.customFont == entry.postScriptName) {
                            custom.wrappedValue = entry.postScriptName
                        }
                        .contextMenu {
                            Button("Remove from app", systemImage: "trash", role: .destructive) {
                                if item.customFont == entry.postScriptName { custom.wrappedValue = nil }
                                library.remove(entry)
                            }
                        }
                    }
                }
            }
            if let fontError { Text(fontError).font(.caption).foregroundStyle(.red) }
            if let name = item.customFont, !library.fonts.contains(where: { $0.postScriptName == name }) {
                Text("“\(name)” isn't installed on this device, so the built-in font below is used.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            }
            SectionLabel("Built in")
            LazyVGrid(columns: fontsGrid, spacing: 8) {
                ForEach(TextFont.allCases) { f in
                    fontTile(sample: Font(TextRasterizer.font(f, size: 19, bold: item.bold)), name: f.label,
                             on: item.customFont == nil && item.font == f) {
                        let id = item.id
                        model.edit { p in
                            if let j = p.texts.firstIndex(where: { $0.id == id }) { p.texts[j].font = f; p.texts[j].customFont = nil }
                        }
                    }
                }
            }
        }
        .fileImporter(isPresented: $importingFont, allowedContentTypes: Self.fontTypes, allowsMultipleSelection: true) { result in
            fontError = nil
            guard case .success(let urls) = result else { return }
            var first: String?
            var failures: [String] = []
            for url in urls {
                do {
                    let faces = try library.importFont(at: url)
                    if first == nil { first = faces.first?.postScriptName }
                } catch { failures.append(error.localizedDescription) }
            }
            if let first { custom.wrappedValue = first }
            if !failures.isEmpty { fontError = failures.joined(separator: "\n") }
        }
    }

    private static let fontTypes: [UTType] = [.font] + ["ttf", "otf", "ttc"].compactMap { UTType(filenameExtension: $0) }

    private func fontTile(sample: Font, name: String, on: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 2) {
                Text("Aa أب").font(sample).lineLimit(1)
                Text(name).font(.system(size: 9.5)).foregroundStyle(Theme.secondaryText).lineLimit(1)
            }
            .padding(.horizontal, 4)
            .frame(maxWidth: .infinity, minHeight: 56)
            .background(on ? Theme.accent.opacity(0.18) : Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(on ? Theme.accent : .clear, lineWidth: 2))
        }
        .buttonStyle(.plain)
    }

    private var animateTab: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionLabel("In")
            Chips(options: TextAnimation.allCases.map { ($0, $0.label) }, selection: model.textBinding(\.animationIn, fallback: .none))
            SectionLabel("Out")
            Chips(options: TextAnimation.allCases.filter { $0 != .typewriter }.map { ($0, $0.label) },
                  selection: model.textBinding(\.animationOut, fallback: .none))
        }
    }

    private func swatches(_ title: String, _ binding: Binding<RGBA>) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            SectionLabel(title)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(RGBA.swatches, id: \.self) { c in
                        Circle().fill(Color(uiColor: c.uiColor))
                            .frame(width: 28, height: 28)
                            .overlay(Circle().stroke(binding.wrappedValue == c ? Theme.accent : .white.opacity(0.25),
                                                     lineWidth: binding.wrappedValue == c ? 3 : 1))
                            .onTapGesture { binding.wrappedValue = c }
                    }
                }
                .padding(2)
            }
        }
    }
}

struct TextTemplatesPanel: View {
    @Bindable var model: EditorModel
    var open: (Panel) -> Void

    static let templates: [(String, TextOverlay)] = [
        ("Bold title", TextOverlay(text: "TITLE", size: 0.11, style: .outline, font: .futura, textCase: .upper, animationIn: .pop)),
        ("Neon", TextOverlay(text: "Neon", color: .white, accent: RGBA(r: 0, g: 0.9, b: 0.85), size: 0.1, style: .glow, font: .rounded, animationIn: .fade)),
        ("Subtitle", TextOverlay(text: "Subtitle", color: .white, accent: RGBA(r: 0, g: 0, b: 0, a: 0.75), size: 0.055, bold: false, style: .box, y: 0.82)),
        ("Lower third", TextOverlay(text: "Name · Title", color: .black, accent: .yellow, size: 0.05, style: .box, animationIn: .slideUp, x: 0.5, y: 0.78)),
        ("Handwritten", TextOverlay(text: "hello", color: .yellow, accent: .black, size: 0.1, style: .shadow, font: .marker, animationIn: .zoom)),
        ("Typewriter", TextOverlay(text: "Once upon a time…", color: .white, accent: .black, size: 0.06, bold: false, style: .shadow, font: .typewriter, animationIn: .typewriter)),
        ("عنوان", TextOverlay(text: "عنوان رئيسي", color: .white, accent: .black, size: 0.1, style: .outline, font: .geeza, animationIn: .pop)),
        ("خط نسخ", TextOverlay(text: "بسم الله", color: RGBA(r: 1, g: 0.86, b: 0.5), accent: .black, size: 0.1, style: .shadow, font: .naskh, animationIn: .fade)),
        ("Pop caption", TextOverlay(text: "WOW", color: .yellow, accent: .black, size: 0.12, style: .outline, font: .avenir, textCase: .upper, animationIn: .bounce)),
        ("Elegant", TextOverlay(text: "Elegant", color: .white, accent: .black, size: 0.09, bold: false, style: .shadow, font: .script, animationIn: .fade, animationOut: .fade)),
    ]

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 140), spacing: 8)], spacing: 8) {
            ForEach(Self.templates, id: \.0) { name, t in
                Button {
                    model.addText(t)
                    open(.text(.edit))
                } label: {
                    VStack(spacing: 4) {
                        Text(t.text)
                            .font(Font(TextRasterizer.font(t.font, size: 20, bold: t.bold)))
                            .foregroundStyle(Color(uiColor: t.color.uiColor))
                            .shadow(color: Color(uiColor: t.accent.uiColor), radius: t.style == .glow ? 6 : 1)
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .background(t.style == .box ? Color(uiColor: t.accent.uiColor) : .clear, in: RoundedRectangle(cornerRadius: 6))
                            .lineLimit(1)
                        Text(name).font(.system(size: 10)).foregroundStyle(Theme.secondaryText)
                    }
                    .frame(maxWidth: .infinity, minHeight: 74)
                    .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
                }
                .buttonStyle(PressStyle())
            }
        }
    }
}

struct ShapesPanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            LazyVGrid(columns: gridColumns, spacing: 8) {
                ForEach(ShapeKind.allCases) { kind in
                    GridPick(icon: kind.symbol, label: kind.rawValue.capitalized, selected: false) { model.addShape(kind) }
                }
            }
            Text("Shapes sit on the text track. Select one to change its colour, size and animation, or drag it on the preview.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
        }
    }
}

struct CaptionsPanel: View {
    @Bindable var model: EditorModel
    @AppStorage("captionLanguage") private var language = "ar-SA"
    @AppStorage("captionWords") private var words = 3
    @State private var working = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Transcribes your clips' sound and adds timed captions. Runs on device when the language is installed, otherwise through Apple's speech service.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
            Chips(options: Captioner.languages.map { ($0.id, $0.name) }, selection: $language)
            Stepper("Up to \(words) word\(words == 1 ? "" : "s") per caption", value: $words, in: 1...8)
                .font(.subheadline)
            HStack {
                Button {
                    Task { await generate() }
                } label: {
                    Label(working ? "Listening…" : "Generate captions", systemImage: "waveform")
                        .font(.subheadline.weight(.bold))
                        .frame(maxWidth: .infinity, minHeight: 40)
                        .background(Theme.accent, in: Capsule())
                        .foregroundStyle(.black)
                }
                .buttonStyle(.plain)
                .disabled(working || model.project.clips.isEmpty)
                if model.project.texts.contains(where: \.isCaption) {
                    Button("Remove", role: .destructive) { model.replaceCaptions([]) }
                        .font(.subheadline.weight(.semibold))
                        .padding(.horizontal, 14).frame(minHeight: 40)
                        .background(Theme.surfaceHigh, in: Capsule())
                        .buttonStyle(.plain)
                }
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
        }
    }

    private func generate() async {
        working = true
        error = nil
        defer { working = false }
        do {
            let captions = try await Captioner.captions(for: model.project, locale: language, wordsPerCaption: words)
            model.replaceCaptions(captions)
        } catch {
            self.error = error.localizedDescription
        }
    }
}
