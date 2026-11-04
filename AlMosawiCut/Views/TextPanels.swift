import SwiftUI
import UniformTypeIdentifiers

struct TextPanel: View {
    @Bindable var model: EditorModel
    @State private var tab: TextTab
    @FocusState private var focused: Bool
    @State private var library = FontLibrary.shared
    @State private var wordColor: RGBA = .yellow
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
            wordColors(item)
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
            if !item.wordStarts.isEmpty {
                Toggle("Light up words as they're spoken", isOn: model.textBinding(\.karaoke, fallback: false))
                    .font(.subheadline)
                if item.karaoke { swatches("Spoken words", model.textBinding(\.karaokeColor, fallback: .yellow)) }
            }
            if item.isCaption {
                Button { model.applyStyleToAllCaptions(from: item) } label: {
                    Label("Use this style for every caption", systemImage: "captions.bubble.fill").font(.caption.weight(.bold))
                }
                .buttonStyle(.plain).foregroundStyle(Theme.accent)
            }
        }
    }

    /// Tap a word to give it its own colour; tap again to clear it.
    @ViewBuilder
    private func wordColors(_ item: TextOverlay) -> some View {
        let words = item.text.split(whereSeparator: \.isWhitespace).map(String.init)
        if words.count > 1 {
            VStack(alignment: .leading, spacing: 6) {
                SectionLabel("Color single words")
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(Array(words.enumerated()), id: \.offset) { i, word in
                            let mark = item.highlights.first { $0.index == i }
                            Button {
                                var list = item.highlights.filter { $0.index != i }
                                if mark == nil { list.append(WordHighlight(index: i, color: wordColor)) }
                                model.textBinding(\.highlights, fallback: []).wrappedValue = list
                            } label: {
                                Text(word)
                                    .font(.caption.weight(.semibold))
                                    .padding(.horizontal, 10).frame(height: 28)
                                    .background(mark.map { Color(uiColor: $0.color.uiColor).opacity(0.35) } ?? Theme.surfaceHigh, in: Capsule())
                                    .overlay(Capsule().stroke(mark.map { Color(uiColor: $0.color.uiColor) } ?? .clear, lineWidth: 2))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                HStack(spacing: 8) {
                    ForEach(RGBA.swatches, id: \.self) { c in
                        Circle().fill(Color(uiColor: c.uiColor)).frame(width: 22, height: 22)
                            .overlay(Circle().stroke(wordColor == c ? Theme.accent : .white.opacity(0.25), lineWidth: wordColor == c ? 3 : 1))
                            .onTapGesture { wordColor = c }
                    }
                }
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
            if let name = item.customFont, !library.isInstalled(name) {
                Text("“\(name)” isn't installed on this device, so the built-in font below is used.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            }
            if !library.bundled.isEmpty {
                SectionLabel("Title fonts")
                LazyVGrid(columns: fontsGrid, spacing: 8) {
                    ForEach(library.bundled) { entry in
                        fontTile(sample: Font(TextRasterizer.customFont(entry.postScriptName, size: 19, bold: false) ?? .systemFont(ofSize: 19)),
                                 name: entry.displayName, on: item.customFont == entry.postScriptName) {
                            custom.wrappedValue = entry.postScriptName
                        }
                    }
                }
            }
            SectionLabel("iOS fonts")
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

    /// Concat's title presets, drawn in the same open-licence fonts (bundled in Fonts/).
    static let concatPresets: [(String, TextOverlay)] = [
        ("Poster", TextOverlay(text: "POSTER", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.144, bold: false, style: .shadow, customFont: "BebasNeue-Regular", tracking: 0.03)),
        ("Bold caption", TextOverlay(text: "Bold caption", color: RGBA(r: 1, g: 1, b: 1), accent: RGBA(r: 0, g: 0, b: 0), size: 0.056, bold: false, style: .outline, customFont: "Montserrat-Black", y: 0.8)),
        ("Comic", TextOverlay(text: "POW!", color: RGBA(r: 1, g: 0.882, b: 0.29), accent: RGBA(r: 0.067, g: 0.067, b: 0.067), size: 0.112, bold: false, style: .outline, customFont: "Bangers-Regular", tracking: 0.02)),
        ("Editorial", TextOverlay(text: "Editorial", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.08, bold: false, style: .plain, customFont: "DMSerifDisplay-Italic")),
        ("Retro", TextOverlay(text: "Retro", color: RGBA(r: 1, g: 0.435, b: 0.71), accent: RGBA(r: 0.227, g: 0.043, b: 0.165), size: 0.08, bold: false, style: .outline, customFont: "Pacifico-Regular")),
        ("Tech", TextOverlay(text: "Tech", color: RGBA(r: 0.608, g: 0.906, b: 1), accent: .black, size: 0.064, bold: false, style: .plain, customFont: "SpaceGrotesk-Medium", tracking: 0.02)),
        ("Block", TextOverlay(text: "Block", color: RGBA(r: 0.067, g: 0.067, b: 0.067), accent: RGBA(r: 1, g: 0.882, b: 0.29), size: 0.056, bold: false, style: .box, customFont: "ArchivoBlack-Regular")),
        ("Marker", TextOverlay(text: "Marker", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.072, bold: false, style: .shadow, customFont: "PermanentMarker-Regular")),
        ("Impact", TextOverlay(text: "IMPACT", color: RGBA(r: 1, g: 1, b: 1), accent: RGBA(r: 0, g: 0, b: 0), size: 0.12, bold: false, style: .outline, customFont: "Anton-Regular")),
        ("Fashion", TextOverlay(text: "Fashion", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.088, bold: false, style: .plain, customFont: "AbrilFatface-Regular")),
        ("Pixel", TextOverlay(text: "Pixel", color: RGBA(r: 0.49, g: 1, b: 0.416), accent: .black, size: 0.072, bold: false, style: .shadow, customFont: "PixelifySans-Regular")),
        ("Lower third", TextOverlay(text: "Name — Title", color: RGBA(r: 0.063, g: 0.086, b: 0.039), accent: RGBA(r: 0.776, g: 0.957, b: 0.196), size: 0.04, bold: false, style: .box, customFont: "HankenGrotesk-SemiBold", y: 0.82)),
        ("Western", TextOverlay(text: "WESTERN", color: RGBA(r: 0.949, g: 0.769, b: 0.427), accent: RGBA(r: 0.231, g: 0.122, b: 0.039), size: 0.072, bold: false, style: .outline, customFont: "Frijole")),
        ("Diner", TextOverlay(text: "Diner", color: RGBA(r: 1, g: 0.302, b: 0.302), accent: RGBA(r: 1, g: 1, b: 1), size: 0.088, bold: false, style: .outline, customFont: "Lobster-Regular")),
        ("Stencil", TextOverlay(text: "STENCIL", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.096, bold: false, style: .shadow, customFont: "Staatliches-Regular", tracking: 0.04)),
        ("Cartoon", TextOverlay(text: "Cartoon", color: RGBA(r: 1, g: 0.541, b: 0.239), accent: RGBA(r: 1, g: 1, b: 1), size: 0.096, bold: false, style: .outline, customFont: "LondrinaSolid-Black")),
        ("Handwritten", TextOverlay(text: "Handwritten", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.08, bold: false, style: .plain, customFont: "CaveatBrush-Regular")),
        ("Code", TextOverlay(text: "code()", color: RGBA(r: 0.902, g: 0.929, b: 0.953), accent: RGBA(r: 0.051, g: 0.067, b: 0.09, a: 0.902), size: 0.04, bold: false, style: .box, customFont: "GoogleSansCode-Regular")),
        ("Mission", TextOverlay(text: "MISSION", color: RGBA(r: 0.784, g: 0.839, b: 0.627), accent: .black, size: 0.08, bold: false, style: .shadow, customFont: "BlackOpsOne-Regular")),
        ("Clean", TextOverlay(text: "Clean", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.064, bold: false, style: .plain, customFont: "Inter-SemiBold")),
        ("Candy", TextOverlay(text: "Candy", color: RGBA(r: 1, g: 0.604, b: 0.835), accent: RGBA(r: 0.353, g: 0.078, b: 0.251), size: 0.08, bold: false, style: .outline, customFont: "EmilysCandy-Regular")),
        ("Atomic", TextOverlay(text: "Atomic", color: RGBA(r: 0.498, g: 1, b: 0.831), accent: .black, size: 0.072, bold: false, style: .shadow, customFont: "AtomicAge-Regular")),
        ("Blackletter", TextOverlay(text: "Headline", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.088, bold: false, style: .shadow, customFont: "GermaniaOne-Regular")),
        ("Zoo", TextOverlay(text: "ZOO", color: RGBA(r: 1, g: 0.824, b: 0.247), accent: RGBA(r: 0.102, g: 0.102, b: 0.102), size: 0.096, bold: false, style: .outline, customFont: "ZenTokyoZoo-Regular")),
        ("Hairline", TextOverlay(text: "HAIRLINE", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.072, bold: false, style: .plain, customFont: "BungeeHairline-Regular")),
        ("Scribble", TextOverlay(text: "Scribble", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.088, bold: false, style: .shadow, customFont: "RubikScribble-Regular")),
        ("Mystery", TextOverlay(text: "Mystery", color: RGBA(r: 0.788, g: 0.714, b: 1), accent: .black, size: 0.08, bold: false, style: .shadow, customFont: "MysteryQuest")),
        ("Friendly", TextOverlay(text: "Friendly", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.064, bold: false, style: .shadow, customFont: "HammersmithOne-Regular")),
        ("Notes", TextOverlay(text: "Notes", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.072, bold: false, style: .plain, customFont: "ShadowsIntoLightTwo-Regular")),
        ("Japanese", TextOverlay(text: "こんにちは", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.072, bold: false, style: .shadow, customFont: "KosugiMaru-Regular")),
        ("Hebrew", TextOverlay(text: "שלום", color: RGBA(r: 1, g: 1, b: 1), accent: .black, size: 0.08, bold: false, style: .shadow, customFont: "SecularOne-Regular")),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionLabel("Titles")
            grid(Self.concatPresets)
            SectionLabel("Styles")
            grid(Self.templates)
        }
    }

    private func grid(_ items: [(String, TextOverlay)]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 140), spacing: 8)], spacing: 8) {
            ForEach(items, id: \.0) { name, t in
                Button {
                    model.addText(t)
                    open(.text(.edit))
                } label: {
                    VStack(spacing: 4) {
                        Text(t.text)
                            .font(Font(TextRasterizer.font(for: t, size: 20)))
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
    @State private var status = ""
    @State private var summary: String?
    @State private var error: String?
    @State private var importingSRT = false
    @AppStorage("captionEngine") private var engineRaw = Captioner.Engine.whisper.rawValue
    @AppStorage("whisperSize") private var sizeRaw = WhisperEngine.Size.small.rawValue
    @State private var whisper = WhisperEngine.shared
    private var engine: Captioner.Engine { Captioner.Engine(rawValue: engineRaw) ?? .whisper }
    private var size: WhisperEngine.Size { WhisperEngine.Size(rawValue: sizeRaw) ?? .small }
    @State private var srtFile: URL?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Makes captions from the sound of the video you're editing (clips, voiceovers and extracted audio, not music), timed to your timeline. Generate again after big changes.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
            Picker("Engine", selection: $engineRaw) {
                ForEach(Captioner.Engine.allCases) { Text($0.label).tag($0.rawValue) }
            }
            .pickerStyle(.segmented)
            if engine == .whisper {
                Text("Whisper runs on this iPhone. No microphone, permissions or Dictation; offline after a one-time model download.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
                SectionLabel("Speech model")
                Chips(options: WhisperEngine.Size.allCases.map { ($0.rawValue, $0.label + (whisper.isDownloaded($0) ? " ✓" : "")) }, selection: $sizeRaw)
                HStack(alignment: .firstTextBaseline) {
                    Text(size.detail + (whisper.isDownloaded(size) ? " Downloaded." : " Downloads on first use."))
                        .font(.caption).foregroundStyle(Theme.secondaryText)
                    Spacer()
                    if whisper.isDownloaded(size), !working {
                        Button("Remove", role: .destructive) { whisper.delete(size) }
                            .font(.caption.weight(.semibold)).buttonStyle(.plain).foregroundStyle(.red)
                    }
                }
                if let p = whisper.downloadProgress {
                    ProgressView(value: p) { Text("Downloading \(Int(p * 100))%").font(.caption) }
                }
            } else {
                Text("Uses Apple's speech recognition: iOS asks for permission once, and Dictation must be on in Settings › General › Keyboard.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            }
            Chips(options: Captioner.languages.map { ($0.id, $0.name) }, selection: $language)
            SectionLabel("Words on screen at once")
            Chips(options: [(1, "1 word"), (3, "3 words"), (5, "5 words"), (8, "8 words"), (20, "Full sentence")], selection: $words)
            Text("Captions cover the whole video; this only sets how much of it shows at a time.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
            HStack {
                Button {
                    Task { await generate() }
                } label: {
                    Label(working ? "Working…" : model.project.texts.contains(where: \.isCaption) ? "Generate again" : "Generate captions",
                          systemImage: "waveform")
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
            HStack(spacing: 8) {
                Button { importingSRT = true } label: {
                    Label("Import .srt", systemImage: "square.and.arrow.down").font(.caption.weight(.bold))
                }
                if let srtFile {
                    ShareLink(item: srtFile) {
                        Label("Share .srt", systemImage: "square.and.arrow.up").font(.caption.weight(.bold))
                    }
                } else if model.project.texts.contains(where: \.isCaption) {
                    Button { srtFile = writeSRT() } label: {
                        Label("Export .srt", systemImage: "doc.text").font(.caption.weight(.bold))
                    }
                }
            }
            .buttonStyle(.bordered)
            .onChange(of: model.project.texts) { _, _ in srtFile = nil }
            .fileImporter(isPresented: $importingSRT, allowedContentTypes: Self.srtTypes) { result in
                guard case .success(let url) = result else { return }
                importSRT(url)
            }
            if working, !status.isEmpty {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(status).font(.caption).foregroundStyle(Theme.secondaryText)
                }
            }
            if let summary, !working {
                Label(summary, systemImage: "checkmark.circle.fill")
                    .font(.caption.weight(.semibold)).foregroundStyle(Theme.accent)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
        }
        .onAppear { if ![1, 3, 5, 8, 20].contains(words) { words = 3 } }
    }

    private static let srtTypes: [UTType] = [UTType(filenameExtension: "srt"), .plainText, .text].compactMap { $0 }

    private func writeSRT() -> URL? {
        let captions = model.project.texts.filter(\.isCaption)
        let name = model.project.name.replacingOccurrences(of: "/", with: "-")
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(name).srt")
        do {
            try SRT.export(captions).write(to: url, atomically: true, encoding: .utf8)
            return url
        } catch {
            model.errorMessage = error.localizedDescription
            return nil
        }
    }

    /// Captions from a subtitle file, styled like auto captions (or like the existing ones).
    private func importSRT(_ url: URL) {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        guard let data = try? Data(contentsOf: url),
              let text = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .isoLatin1) else {
            model.errorMessage = "Couldn't read that subtitle file."
            return
        }
        let cues = SRT.parse(text)
        guard !cues.isEmpty else { model.errorMessage = "No subtitles were found in that file."; return }
        let look = model.project.texts.first(where: \.isCaption)
            ?? Captioner.group([Captioner.Word(text: "x", start: 0, duration: 1)], max: 1)[0]
        let captions = cues.map { cue -> TextOverlay in
            var c = look
            c.id = UUID(); c.text = cue.text; c.start = cue.start; c.duration = cue.end - cue.start; c.isCaption = true
            c.wordStarts = []; c.highlights = []; c.karaoke = false
            return c
        }
        model.replaceCaptions(captions)
        summary = "Imported \(captions.count) caption\(captions.count == 1 ? "" : "s") from \(url.lastPathComponent)"
        if let first = captions.first { model.seek(to: first.start + 0.05) }
    }

    private func generate() async {
        working = true
        summary = nil
        error = nil
        defer { working = false; status = "" }
        if model.isPlaying { model.togglePlay() }
        do {
            let captions = try await Captioner.captions(for: model.project, locale: language, wordsPerCaption: words,
                                                        engine: engine, model: size) { status = $0 }
            model.replaceCaptions(captions)
            if let first = captions.first, let last = captions.last {
                // Show the first caption on the preview straight away.
                model.selection = nil
                model.seek(to: min(first.start + 0.05, max(0, model.duration - 0.05)))
                summary = "Added \(captions.count) caption\(captions.count == 1 ? "" : "s"), from \(first.start.timecode) to \(last.end.timecode) of \(model.duration.timecode)"
            }
        } catch {
            self.error = error.localizedDescription
            model.errorMessage = "Couldn't make captions: \(error.localizedDescription)"
        }
    }
}
