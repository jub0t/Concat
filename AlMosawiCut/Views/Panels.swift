import AVFoundation
import SwiftUI

/// The inline editors that replace the toolbar while a tool is open.
struct PanelContent: View {
    let panel: Panel
    @Bindable var model: EditorModel
    var open: (Panel) -> Void

    var body: some View {
        switch panel {
        case .speed: SpeedPanel(model: model)
        case .volume: VolumePanel(model: model)
        case .animate: AnimatePanel(model: model)
        case .crop: CropPanel(model: model)
        case .filters: FilterPanel(model: model)
        case .effects: EffectsPanel(model: model)
        case .adjust: AdjustPanel(model: model)
        case .cutout: CutoutPanel(model: model)
        case .voiceFX: VoiceFXPanel(model: model)
        case .fade: FadePanel(model: model)
        case .trim: TrimPanel(model: model)
        case .transition(let i): TransitionPanel(model: model, index: i)
        case .blend: BlendPanel(model: model)
        case .mask: MaskPanel(model: model)
        case .text(let tab): TextPanel(model: model, initialTab: tab)
        case .textTemplates: TextTemplatesPanel(model: model, open: open)
        case .shapes: ShapesPanel(model: model)
        case .captions: CaptionsPanel(model: model)
        case .tts: SpeechPanel(model: model)
        case .music: MusicPanel(model: model, open: open)
        case .record: RecordPanel(model: model)
        case .audioItem: AudioItemPanel(model: model)
        case .canvas: CanvasPanel(model: model)
        }
    }
}

// MARK: Building blocks

struct LabeledSlider: View {
    let title: String
    @Binding var value: Double
    let range: ClosedRange<Double>
    var format: (Double) -> String = { String(format: "%.2f", $0) }
    var reset: Double?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(title).font(.caption.weight(.medium)).foregroundStyle(Theme.secondaryText)
                Spacer()
                Text(format(value)).font(.caption.monospacedDigit().weight(.semibold))
                if let reset, abs(value - reset) > 0.0001 {
                    Button { value = reset } label: {
                        Image(systemName: "arrow.counterclockwise").font(.caption2.weight(.bold))
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(Theme.accent)
                }
            }
            Slider(value: Binding(get: { min(max(value, range.lowerBound), range.upperBound) }, set: { value = $0 }), in: range)
        }
    }
}

struct Chips<T: Hashable>: View {
    let options: [(T, String)]
    @Binding var selection: T

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(options, id: \.0) { option in
                    let on = option.0 == selection
                    Button { selection = option.0 } label: {
                        Text(option.1)
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 12).frame(height: 30)
                            .background(on ? Theme.accent : Theme.surfaceHigh, in: Capsule())
                            .foregroundStyle(on ? .black : .white)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }
}

struct GridPick: View {
    let icon: String
    let label: String
    let selected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.system(size: 18, weight: .medium))
                    .frame(width: 54, height: 46)
                    .background(selected ? Theme.accent.opacity(0.22) : Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(selected ? Theme.accent : .clear, lineWidth: 2))
                Text(label)
                    .font(.system(size: 10, weight: .medium))
                    .lineLimit(2)
                    .multilineTextAlignment(.center)
                    .frame(height: 26, alignment: .top)
            }
            .frame(width: 68)
            .foregroundStyle(selected ? Theme.accent : .white)
        }
        .buttonStyle(PressStyle())
    }
}

struct SectionLabel: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text.uppercased()).font(.system(size: 10.5, weight: .bold)).foregroundStyle(Theme.secondaryText)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct NeedsSelection: View {
    let text: String
    var body: some View {
        Text(text).font(.subheadline).foregroundStyle(Theme.secondaryText).frame(maxWidth: .infinity).padding(.top, 20)
    }
}

let gridColumns = [GridItem(.adaptive(minimum: 68), spacing: 6)]

// MARK: Speed, volume, fade, trim

private struct SpeedPanel: View {
    @Bindable var model: EditorModel
    @State private var curveMode = false
    private let presets: [Double] = [0.25, 0.5, 1, 1.5, 2, 3, 5]

    var body: some View {
        if let clip = model.selectedMedia {
            let speed = model.mediaBinding(\.speed, fallback: 1)
            let curve = model.mediaBinding(\.speedCurve, fallback: .none)
            VStack(alignment: .leading, spacing: 14) {
                Picker("", selection: $curveMode) {
                    Text("Normal").tag(false)
                    Text("Curve").tag(true)
                }
                .pickerStyle(.segmented)
                if curveMode {
                    LazyVGrid(columns: gridColumns, spacing: 10) {
                        ForEach(SpeedCurve.allCases) { c in
                            Button { curve.wrappedValue = c } label: {
                                VStack(spacing: 6) {
                                    CurveGraph(curve: c)
                                        .stroke(curve.wrappedValue == c ? Theme.accent : .white, lineWidth: 2)
                                        .frame(width: 50, height: 32)
                                        .padding(7)
                                        .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
                                        .overlay(RoundedRectangle(cornerRadius: 12).stroke(curve.wrappedValue == c ? Theme.accent : .clear, lineWidth: 2))
                                    Text(c.label).font(.system(size: 10, weight: .medium))
                                }
                                .frame(width: 68)
                            }
                            .buttonStyle(PressStyle())
                        }
                    }
                    if curve.wrappedValue == .custom {
                        Text("Drag the points: up is faster, down is slower.").font(.caption).foregroundStyle(Theme.secondaryText)
                        SpeedCurveEditor(points: model.mediaBinding(\.speedPoints, fallback: [1, 1, 1, 1, 1, 1]))
                            .frame(height: 140)
                    }
                } else {
                    HStack(spacing: 6) {
                        ForEach(presets, id: \.self) { p in
                            let on = abs(speed.wrappedValue - p) < 0.001
                            Button(String(format: "%gx", p)) { speed.wrappedValue = p }
                                .font(.caption.weight(.bold))
                                .frame(maxWidth: .infinity, minHeight: 32)
                                .background(on ? Theme.accent : Theme.surfaceHigh, in: Capsule())
                                .foregroundStyle(on ? .black : .white)
                                .buttonStyle(.plain)
                        }
                    }
                    LabeledSlider(title: "Speed", value: speed, range: 0.1...10, format: { String(format: "%.2fx", $0) }, reset: 1)
                }
                Text("Length on timeline: \(clip.duration.timecode)").font(.caption).foregroundStyle(Theme.secondaryText)
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }
}

/// Six draggable speed points on a log scale from 0.1x to 10x.
struct SpeedCurveEditor: View {
    @Binding var points: [Double]

    private func y(_ speed: Double, _ h: CGFloat) -> CGFloat {
        let v = (log10(max(0.1, min(10, speed))) + 1) / 2  // 0.1x -> 0, 10x -> 1
        return h * (1 - CGFloat(v))
    }

    private func speed(atY yy: CGFloat, _ h: CGFloat) -> Double {
        let v = 1 - Double(max(0, min(h, yy)) / max(1, h))
        let s = pow(10, v * 2 - 1)
        return (s * 20).rounded() / 20
    }

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width, h = geo.size.height
            let n = max(2, points.count)
            let xs = (0..<n).map { CGFloat($0) / CGFloat(n - 1) * (w - 24) + 12 }
            ZStack {
                RoundedRectangle(cornerRadius: 12).fill(Theme.surfaceHigh)
                Path { p in p.move(to: CGPoint(x: 0, y: y(1, h))); p.addLine(to: CGPoint(x: w, y: y(1, h))) }
                    .stroke(Color.white.opacity(0.2), style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
                Path { p in
                    for i in 0...60 {
                        let t = Double(i) / 60
                        let pt = CGPoint(x: 12 + CGFloat(t) * (w - 24), y: y(Clip.interpolate(points, at: t), h))
                        i == 0 ? p.move(to: pt) : p.addLine(to: pt)
                    }
                }
                .stroke(Theme.accent, lineWidth: 2.5)
                ForEach(0..<n, id: \.self) { i in
                    let s = i < points.count ? points[i] : 1
                    Circle().fill(Color.white).frame(width: 22, height: 22)
                        .overlay(Text(String(format: "%.2g", s)).font(.system(size: 7, weight: .bold)).foregroundStyle(.black))
                        .position(x: xs[i], y: y(s, h))
                        .gesture(DragGesture(minimumDistance: 0).onChanged { v in
                            guard i < points.count else { return }
                            points[i] = speed(atY: v.location.y, h)
                        })
                }
                VStack {
                    HStack { Text("10x"); Spacer() }
                    Spacer()
                    HStack { Text("0.1x"); Spacer() }
                }
                .font(.system(size: 9)).foregroundStyle(Theme.secondaryText).padding(6).allowsHitTesting(false)
            }
        }
    }
}

struct CurveGraph: Shape {
    let curve: SpeedCurve
    func path(in rect: CGRect) -> Path {
        var p = Path()
        for i in 0...40 {
            let x = Double(i) / 40
            let s = curve.speed(at: x)
            let y = 1 - min(1, log2(max(0.2, s)) / 4 + 0.5)
            let pt = CGPoint(x: rect.minX + rect.width * x, y: rect.minY + rect.height * y)
            i == 0 ? p.move(to: pt) : p.addLine(to: pt)
        }
        return p
    }
}

private struct VolumePanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if model.selectedMedia != nil {
            let volume = model.mediaBinding(\.volume, fallback: 1)
            VStack(alignment: .leading, spacing: 14) {
                LabeledSlider(title: "Volume", value: volume, range: 0...2, format: { "\(Int($0 * 100))%" }, reset: 1)
                Button {
                    volume.wrappedValue = volume.wrappedValue == 0 ? 1 : 0
                } label: {
                    Label(volume.wrappedValue == 0 ? "Unmute" : "Mute", systemImage: volume.wrappedValue == 0 ? "speaker.wave.2.fill" : "speaker.slash.fill")
                        .font(.subheadline.weight(.semibold))
                        .padding(.horizontal, 16).frame(height: 36)
                        .background(Theme.surfaceHigh, in: Capsule())
                }
                .buttonStyle(.plain)
                Text("Volume can also change over time: add keyframes in Animate and set the volume there.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
                LabeledSlider(title: "Volume at playhead (keyframe)", value: model.transformBinding(\.volume), range: 0...2,
                              format: { "\(Int($0 * 100))%" }, reset: 1)
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }
}

private struct FadePanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if let clip = model.selectedMedia {
            let limit = max(0.1, clip.duration / 2)
            VStack(spacing: 12) {
                LabeledSlider(title: "Fade in", value: model.mediaBinding(\.fadeIn, fallback: 0), range: 0...limit,
                              format: { String(format: "%.1fs", $0) }, reset: 0)
                LabeledSlider(title: "Fade out", value: model.mediaBinding(\.fadeOut, fallback: 0), range: 0...limit,
                              format: { String(format: "%.1fs", $0) }, reset: 0)
                Text("Fades the picture to black and the sound to silence.").font(.caption).foregroundStyle(Theme.secondaryText)
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }
}

private struct TrimPanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if let clip = model.selectedMedia {
            let start = model.mediaBinding(\.trimStart, fallback: 0)
            let end = model.mediaBinding(\.trimEnd, fallback: 1)
            VStack(alignment: .leading, spacing: 12) {
                LabeledSlider(title: "Start", value: Binding(get: { start.wrappedValue }, set: { start.wrappedValue = min($0, end.wrappedValue - 0.1) }),
                              range: 0...clip.sourceDuration, format: { $0.timecode })
                LabeledSlider(title: "End", value: Binding(get: { end.wrappedValue }, set: { end.wrappedValue = max($0, start.wrappedValue + 0.1) }),
                              range: 0...clip.sourceDuration, format: { $0.timecode })
                if model.selectedIsOverlay {
                    LabeledSlider(title: "Starts on timeline at", value: model.overlayBinding(\.start, fallback: 0),
                                  range: 0...max(0.1, model.duration), format: { $0.timecode })
                }
                Text("Length on timeline: \(clip.duration.timecode)").font(.caption).foregroundStyle(Theme.secondaryText)
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }
}

// MARK: Animate and crop

private struct AnimatePanel: View {
    @Bindable var model: EditorModel

    var body: some View {
        if let clip = model.selectedMedia {
            VStack(alignment: .leading, spacing: 12) {
                SectionLabel("Quick animations")
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(QuickMotion.allCases, id: \.self) { m in
                            GridPick(icon: m.symbol, label: m.label, selected: false) { apply(m, clip: clip) }
                        }
                    }
                }
                HStack {
                    Image(systemName: "diamond.fill").foregroundStyle(Theme.accent).font(.caption)
                    Text(clip.keyframes.isEmpty ? "No keyframes. Tap ◆ above the timeline to add one at the playhead."
                         : "\(clip.keyframes.count) keyframes. Changing a value records one at the playhead.")
                        .font(.caption).foregroundStyle(Theme.secondaryText)
                    Spacer()
                    if !clip.keyframes.isEmpty {
                        Button("Clear") { model.clearKeyframes() }.font(.caption.weight(.bold)).buttonStyle(.plain).foregroundStyle(Theme.accent)
                    }
                }
                if let easing = model.keyframeEasingBinding {
                    SectionLabel("Curve to the next keyframe")
                    Chips(options: KeyframeEasing.allCases.map { ($0, $0.label) }, selection: easing)
                }
                LabeledSlider(title: "Scale", value: model.transformBinding(\.scale), range: 0.1...4, format: { "\(Int($0 * 100))%" }, reset: 1)
                LabeledSlider(title: "Position X", value: model.transformBinding(\.x), range: -1...1, format: { String(format: "%.2f", $0) }, reset: 0)
                LabeledSlider(title: "Position Y", value: model.transformBinding(\.y), range: -1...1, format: { String(format: "%.2f", $0) }, reset: 0)
                LabeledSlider(title: "Rotation", value: model.transformBinding(\.rotation), range: -180...180, format: { "\(Int($0))°" }, reset: 0)
                LabeledSlider(title: "Opacity", value: model.transformBinding(\.opacity), range: 0...1, format: { "\(Int($0 * 100))%" }, reset: 1)
                Text("Tip: with a clip selected you can also drag, pinch and rotate it right on the preview.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            }
        } else { NeedsSelection(text: "Select a clip or overlay first.") }
    }

    private func apply(_ m: QuickMotion, clip: Clip) {
        let d = clip.duration
        let base = clip.keyframes.first?.value ?? clip.transform
        var a = base, b = base
        switch m {
        case .zoomIn: b.scale = base.scale * 1.3
        case .zoomOut: a.scale = base.scale * 1.3
        case .panLeft: a.scale = base.scale * 1.2; b.scale = a.scale; a.x = 0.08; b.x = -0.08
        case .panRight: a.scale = base.scale * 1.2; b.scale = a.scale; a.x = -0.08; b.x = 0.08
        case .kenBurns: b.scale = base.scale * 1.25; b.x = base.x + 0.05; b.y = base.y - 0.04
        case .spinIn: a.rotation = -180; a.scale = base.scale * 0.2
        case .fadeIn: a.opacity = 0
        case .slideUp: a.y = base.y + 0.6
        case .none:
            model.clearKeyframes(); return
        }
        let first = Keyframe(time: 0, value: a)
        let second = Keyframe(time: [.spinIn, .fadeIn, .slideUp].contains(m) ? min(0.6, d) : d, value: b)
        model.editMedia { $0.keyframes = [first, second] }
    }
}

private enum QuickMotion: CaseIterable {
    case none, zoomIn, zoomOut, panLeft, panRight, kenBurns, spinIn, fadeIn, slideUp
    var label: String {
        switch self {
        case .none: "None"
        case .zoomIn: "Zoom in"
        case .zoomOut: "Zoom out"
        case .panLeft: "Pan left"
        case .panRight: "Pan right"
        case .kenBurns: "Ken Burns"
        case .spinIn: "Spin in"
        case .fadeIn: "Fade in"
        case .slideUp: "Slide up"
        }
    }
    var symbol: String {
        switch self {
        case .none: "nosign"
        case .zoomIn: "plus.magnifyingglass"
        case .zoomOut: "minus.magnifyingglass"
        case .panLeft: "arrow.left"
        case .panRight: "arrow.right"
        case .kenBurns: "camera.metering.center.weighted"
        case .spinIn: "arrow.clockwise"
        case .fadeIn: "circle.lefthalf.filled"
        case .slideUp: "arrow.up"
        }
    }
}

private struct CropPanel: View {
    @Bindable var model: EditorModel
    @State private var sourceAspect: Double = 9.0 / 16.0
    private let ratios: [(String, Double?)] = [("Free", nil), ("1:1", 1), ("9:16", 9.0 / 16), ("16:9", 16.0 / 9), ("4:5", 0.8), ("3:4", 0.75), ("4:3", 4.0 / 3)]

    var body: some View {
        if let clip = model.selectedMedia {
            let crop = model.mediaBinding(\.crop, fallback: CropRect())
            VStack(alignment: .leading, spacing: 12) {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(ratios, id: \.0) { r in
                            Button(r.0) { crop.wrappedValue = rect(for: r.1) }
                                .font(.caption.weight(.semibold))
                                .padding(.horizontal, 12).frame(height: 30)
                                .background(Theme.surfaceHigh, in: Capsule())
                                .buttonStyle(.plain)
                        }
                    }
                }
                LabeledSlider(title: "Left", value: edge(crop, \.x, opposite: false), range: 0...0.45, format: pct, reset: 0)
                LabeledSlider(title: "Right", value: right(crop), range: 0...0.45, format: pct, reset: 0)
                LabeledSlider(title: "Top", value: edge(crop, \.y, opposite: false), range: 0...0.45, format: pct, reset: 0)
                LabeledSlider(title: "Bottom", value: bottom(crop), range: 0...0.45, format: pct, reset: 0)
            }
            .task(id: clip.file) {
                let url = ProjectStore.mediaFolder(for: model.project.id).appendingPathComponent(clip.file)
                if let track = try? await AVURLAsset(url: url).loadTracks(withMediaType: .video).first,
                   let (size, t) = try? await track.load(.naturalSize, .preferredTransform) {
                    let r = size.applying(t)
                    sourceAspect = abs(r.width) / max(1, abs(r.height))
                }
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }

    private func pct(_ v: Double) -> String { "\(Int(v * 100))%" }

    private func rect(for target: Double?) -> CropRect {
        guard let target else { return CropRect() }
        if sourceAspect > target {
            let w = target / sourceAspect
            return CropRect(x: (1 - w) / 2, y: 0, w: w, h: 1)
        } else {
            let h = sourceAspect / target
            return CropRect(x: 0, y: (1 - h) / 2, w: 1, h: h)
        }
    }

    private func edge(_ crop: Binding<CropRect>, _ key: WritableKeyPath<CropRect, Double>, opposite: Bool) -> Binding<Double> {
        Binding(get: { crop.wrappedValue[keyPath: key] }, set: { v in
            var c = crop.wrappedValue
            let sizeKey: WritableKeyPath<CropRect, Double> = key == \.x ? \.w : \.h
            let far = c[keyPath: key] + c[keyPath: sizeKey]
            c[keyPath: key] = min(v, far - 0.1)
            c[keyPath: sizeKey] = far - c[keyPath: key]
            crop.wrappedValue = c
        })
    }

    private func right(_ crop: Binding<CropRect>) -> Binding<Double> {
        Binding(get: { 1 - crop.wrappedValue.x - crop.wrappedValue.w }, set: { v in
            var c = crop.wrappedValue; c.w = max(0.1, 1 - v - c.x); crop.wrappedValue = c
        })
    }

    private func bottom(_ crop: Binding<CropRect>) -> Binding<Double> {
        Binding(get: { 1 - crop.wrappedValue.y - crop.wrappedValue.h }, set: { v in
            var c = crop.wrappedValue; c.h = max(0.1, 1 - v - c.y); crop.wrappedValue = c
        })
    }
}

// MARK: Looks

private struct FilterPanel: View {
    @Bindable var model: EditorModel
    @State private var category = "All"
    @State private var thumbs = Thumbnails.shared

    var body: some View {
        if let clip = model.selectedMedia {
            let filter = model.mediaBinding(\.filter, fallback: "none")
            let intensity = model.mediaBinding(\.filterIntensity, fallback: 1)
            let preview = thumbs.frame(clip.file, at: clip.trimStart)
            let looks = LookCatalog.all.filter { category == "All" || $0.category == category }
            VStack(alignment: .leading, spacing: 12) {
                Chips(options: (["All"] + LookCatalog.categories).map { ($0, $0) }, selection: $category)
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHStack(spacing: 8) {
                        swatch(nil, name: "Original", image: preview, selected: filter.wrappedValue == "none") { filter.wrappedValue = "none" }
                        ForEach(looks, id: \.id) { look in
                            swatch(look, name: look.name, image: preview, selected: filter.wrappedValue == look.id) { filter.wrappedValue = look.id }
                        }
                    }
                }
                .frame(height: 96)
                if filter.wrappedValue != "none" {
                    LabeledSlider(title: "Intensity", value: intensity, range: 0...1, format: { "\(Int($0 * 100))%" }, reset: 1)
                }
                Button {
                    let f = filter.wrappedValue, s = intensity.wrappedValue
                    model.edit { p in for j in p.clips.indices { p.clips[j].filter = f; p.clips[j].filterIntensity = s } }
                } label: {
                    Label("Apply to all clips", systemImage: "square.stack.3d.up.fill").font(.caption.weight(.bold))
                }
                .buttonStyle(.plain).foregroundStyle(Theme.accent)
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }

    private func swatch(_ look: Look?, name: String, image: UIImage?, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 5) {
                LookSwatch(image: image, look: look)
                    .frame(width: 62, height: 62)
                    .clipShape(RoundedRectangle(cornerRadius: 10))
                    .overlay(RoundedRectangle(cornerRadius: 10).stroke(selected ? Theme.accent : .clear, lineWidth: 2.5))
                Text(name).font(.system(size: 10, weight: .medium)).lineLimit(1).frame(width: 66)
                    .foregroundStyle(selected ? Theme.accent : .white)
            }
        }
        .buttonStyle(PressStyle())
    }
}

/// A thumbnail run through a look, so each choice shows what it does.
struct LookSwatch: View {
    let image: UIImage?
    let look: Look?
    @State private var rendered: UIImage?
    private static let context = CIContext()

    var body: some View {
        ZStack {
            Theme.surfaceHigh
            if let shown = rendered ?? image { Image(uiImage: shown).resizable().scaledToFill() }
        }
        .task(id: image) {
            guard let image, let look, let ci = CIImage(image: image) else { return }
            let out = Imaging.apply(look, to: ci, time: 0).cropped(to: ci.extent)
            if let cg = Self.context.createCGImage(out, from: ci.extent) { rendered = UIImage(cgImage: cg) }
        }
    }
}

private struct EffectsPanel: View {
    @Bindable var model: EditorModel
    @State private var category = VideoEffect.categories[0]

    var body: some View {
        if let clip = model.selectedMedia {
            VStack(alignment: .leading, spacing: 12) {
                if !clip.effects.isEmpty {
                    SectionLabel("Applied")
                    ForEach(clip.effects) { e in
                        HStack(alignment: .bottom, spacing: 10) {
                            LabeledSlider(title: e.kind.label + (e.keys.isEmpty ? "" : " · \(e.keys.count) keyframes"),
                                          value: model.effectAmountBinding(e.id), range: 0...1, format: { "\(Int($0 * 100))" })
                            Button { model.toggleEffectKey(e.id) } label: {
                                Image(systemName: model.effectHasKey(e.id) ? "diamond.fill" : "diamond")
                                    .font(.caption.weight(.bold)).frame(width: 30, height: 30)
                                    .background(Theme.surfaceHigh, in: Circle())
                            }
                            .buttonStyle(.plain).foregroundStyle(model.effectHasKey(e.id) ? Theme.accent : .white)
                            .accessibilityLabel("Keyframe this effect at the playhead")
                            Button { model.editMedia { $0.effects.removeAll { $0.id == e.id } } } label: {
                                Image(systemName: "trash").font(.caption.weight(.bold)).frame(width: 30, height: 30)
                                    .background(Theme.surfaceHigh, in: Circle())
                            }
                            .buttonStyle(.plain).foregroundStyle(.red)
                        }
                    }
                }
                if !clip.effects.isEmpty {
                    Text("◆ records the strength at the playhead; with keyframes, the effect changes over time.")
                        .font(.caption).foregroundStyle(Theme.secondaryText)
                }
                SectionLabel("Add effect")
                Chips(options: VideoEffect.categories.map { ($0, $0) }, selection: $category)
                LazyVGrid(columns: gridColumns, spacing: 8) {
                    ForEach(VideoEffect.allCases.filter { $0.category == category }) { kind in
                        let on = clip.effects.contains { $0.kind == kind }
                        GridPick(icon: kind.symbol, label: kind.label, selected: on) {
                            model.editMedia { c in
                                if let i = c.effects.firstIndex(where: { $0.kind == kind }) { c.effects.remove(at: i) }
                                else { c.effects.append(AppliedEffect(kind: kind, amount: kind.defaultAmount)) }
                            }
                        }
                    }
                }
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }

    private func amount(_ id: UUID) -> Binding<Double> {
        Binding(get: { model.selectedMedia?.effects.first { $0.id == id }?.amount ?? 0.5 },
                set: { v in model.editMedia { c in if let i = c.effects.firstIndex(where: { $0.id == id }) { c.effects[i].amount = v } } })
    }
}

private struct AdjustPanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if model.selectedMedia != nil {
            VStack(spacing: 10) {
                row("Brightness", \.brightness, -0.5...0.5, 0)
                row("Contrast", \.contrast, 0.5...1.8, 1)
                row("Saturation", \.saturation, 0...2, 1)
                row("Exposure", \.exposure, -1...1, 0)
                row("Temperature", \.temperature, -1...1, 0)
                row("Tint", \.tint, -1...1, 0)
                row("Highlights", \.highlights, 0...1, 1)
                row("Shadows", \.shadows, -1...1, 0)
                row("Sharpen", \.sharpness, 0...1, 0)
                row("Vignette", \.vignette, 0...1, 0)
                SectionLabel("Color range")
                Chips(options: ColorRangeMode.allCases.map { ($0, $0.label) }, selection: model.mediaBinding(\.colorRange, fallback: .auto))
                SectionLabel("Color wheels")
                wheel("Shadows", \.wheels.shadows)
                wheel("Midtones", \.wheels.midtones)
                wheel("Highlights", \.wheels.highlights)
                Button("Reset all") {
                    model.editMedia { c in
                        c.brightness = 0; c.contrast = 1; c.saturation = 1; c.exposure = 0; c.temperature = 0
                        c.tint = 0; c.highlights = 1; c.shadows = 0; c.sharpness = 0; c.vignette = 0
                        c.wheels = ColorWheels(); c.colorRange = .auto
                    }
                }
                .font(.caption.weight(.bold)).buttonStyle(.plain).foregroundStyle(Theme.accent)
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }

    /// A tint colour and how strongly it applies to one tonal range.
    private func wheel(_ title: String, _ key: WritableKeyPath<Clip, RGBA>) -> some View {
        let value = model.mediaBinding(key, fallback: ColorWheels.neutral)
        let color = Binding<Color>(
            get: { Color(uiColor: UIColor(red: value.wrappedValue.r, green: value.wrappedValue.g, blue: value.wrappedValue.b, alpha: 1)) },
            set: { c in
                var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
                UIColor(c).getRed(&r, green: &g, blue: &b, alpha: &a)
                var v = value.wrappedValue
                v.r = Double(r); v.g = Double(g); v.b = Double(b)
                if v.a == 0 { v.a = 0.5 }
                value.wrappedValue = v
            })
        let strength = Binding<Double>(get: { value.wrappedValue.a }, set: { var v = value.wrappedValue; v.a = $0; value.wrappedValue = v })
        return HStack(alignment: .bottom, spacing: 12) {
            ColorPicker("", selection: color, supportsOpacity: false).labelsHidden()
            LabeledSlider(title: title, value: strength, range: 0...1, format: { "\(Int($0 * 100))%" }, reset: 0)
        }
    }

    private func row(_ title: String, _ key: WritableKeyPath<Clip, Double>, _ range: ClosedRange<Double>, _ neutral: Double) -> some View {
        LabeledSlider(title: title, value: model.mediaBinding(key, fallback: neutral), range: range,
                      format: { String(format: "%+.0f", ($0 - neutral) * 100) }, reset: neutral)
    }
}

private struct CutoutPanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if let clip = model.selectedMedia {
            let mode = model.mediaBinding(\.cutout, fallback: .none)
            VStack(alignment: .leading, spacing: 12) {
                LazyVGrid(columns: gridColumns, spacing: 8) {
                    ForEach(CutoutMode.allCases) { m in
                        GridPick(icon: icon(m), label: m.short, selected: clip.cutout == m) { mode.wrappedValue = m }
                    }
                }
                Text(clip.cutout.label + (clip.cutout == .person || clip.cutout == .subject
                    ? ". Uses on-device Vision on every frame." : ""))
                    .font(.caption).foregroundStyle(Theme.secondaryText)
                if !model.selectedIsOverlay {
                    SectionLabel("Canvas behind the cut-out")
                    Chips(options: CanvasBackground.allCases.map { ($0, $0.label) },
                          selection: Binding(get: { model.project.background }, set: { v in model.edit { $0.background = v } }))
                } else {
                    Text("Tip: put a cut-out person over another video as an overlay.").font(.caption).foregroundStyle(Theme.secondaryText)
                }
            }
        } else { NeedsSelection(text: "Select a clip first.") }
    }

    private func icon(_ m: CutoutMode) -> String {
        switch m {
        case .none: "nosign"
        case .person: "person.fill"
        case .subject: "cube.fill"
        case .greenScreen: "rectangle.fill.badge.checkmark"
        case .blueScreen: "rectangle.fill"
        case .lumaKey: "sun.max.fill"
        case .removeBlack: "moon.fill"
        }
    }
}

// MARK: Transitions, blend, mask

private struct TransitionPanel: View {
    @Bindable var model: EditorModel
    let index: Int

    var body: some View {
        if model.project.clips.indices.contains(index), index > 0 {
            let current = model.project.clips[index].transitionIn
            let kind = Binding(get: { current.kind }, set: { k in
                var t = current; t.kind = k; model.setTransition(t, forClipAt: index)
            })
            let duration = Binding(get: { current.duration }, set: { d in
                var t = current; t.duration = d; model.setTransition(t, forClipAt: index)
            })
            VStack(alignment: .leading, spacing: 12) {
                Text("Between clip \(index) and clip \(index + 1)").font(.caption).foregroundStyle(Theme.secondaryText)
                LazyVGrid(columns: gridColumns, spacing: 8) {
                    ForEach(TransitionKind.allCases) { k in
                        GridPick(icon: k.symbol, label: k.label, selected: current.kind == k) { kind.wrappedValue = k }
                    }
                }
                if current.kind != .none {
                    LabeledSlider(title: "Duration", value: duration, range: 0.1...2, format: { String(format: "%.1fs", $0) }, reset: 0.5)
                }
                Button { model.applyTransitionToAll(current) } label: {
                    Label("Apply to all cuts", systemImage: "square.stack.3d.up.fill").font(.caption.weight(.bold))
                }
                .buttonStyle(.plain).foregroundStyle(Theme.accent)
            }
        } else { NeedsSelection(text: "Add at least two clips to use transitions.") }
    }
}

private struct BlendPanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if model.selectedIsOverlay {
            let blend = model.overlayBinding(\.blend, fallback: .normal)
            VStack(alignment: .leading, spacing: 12) {
                LazyVGrid(columns: gridColumns, spacing: 8) {
                    ForEach(BlendMode.allCases) { m in
                        GridPick(icon: "square.on.square.intersection.dashed", label: m.label, selected: blend.wrappedValue == m) { blend.wrappedValue = m }
                    }
                }
                LabeledSlider(title: "Opacity", value: model.transformBinding(\.opacity), range: 0...1, format: { "\(Int($0 * 100))%" }, reset: 1)
            }
        } else { NeedsSelection(text: "Select an overlay first.") }
    }
}

private struct MaskPanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if model.selectedIsOverlay {
            let mask = model.overlayBinding(\.mask, fallback: .none)
            VStack(alignment: .leading, spacing: 12) {
                LazyVGrid(columns: gridColumns, spacing: 8) {
                    ForEach(MaskShape.allCases) { m in
                        GridPick(icon: icon(m), label: m.label, selected: mask.wrappedValue == m) { mask.wrappedValue = m }
                    }
                }
                if mask.wrappedValue != .none {
                    LabeledSlider(title: "Feather", value: model.overlayBinding(\.maskFeather, fallback: 0), range: 0...1,
                                  format: { "\(Int($0 * 100))%" }, reset: 0)
                }
            }
        } else { NeedsSelection(text: "Select an overlay first.") }
    }

    private func icon(_ m: MaskShape) -> String {
        switch m {
        case .none: "nosign"
        case .circle: "circle.fill"
        case .rectangle: "rectangle.fill"
        case .rounded: "app.fill"
        case .heart: "heart.fill"
        case .star: "star.fill"
        }
    }
}

// MARK: Canvas

private struct CanvasPanel: View {
    @Bindable var model: EditorModel
    @State private var name = ""
    @AppStorage("rippleEdits") private var ripple = true
    @Environment(ProjectStore.self) private var store
    @State private var timelineName = ""
    @State private var templateName = ""
    @State private var naming = false
    @State private var savedTemplate = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            timelines
            SectionLabel("Aspect ratio")
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(AspectRatio.allCases) { aspect in
                        let on = model.project.aspect == aspect
                        Button { model.setAspect(aspect) } label: {
                            VStack(spacing: 6) {
                                RoundedRectangle(cornerRadius: 3)
                                    .stroke(on ? Theme.accent : .white, lineWidth: 2)
                                    .frame(width: 30 * min(1, aspect.ratio), height: 30 / max(1, aspect.ratio))
                                    .frame(width: 44, height: 34)
                                Text(aspect.rawValue).font(.caption2.weight(.bold))
                            }
                            .frame(width: 58, height: 66)
                            .background(on ? Theme.accent.opacity(0.15) : Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(on ? Theme.accent : .white)
                    }
                }
            }
            SectionLabel("Background")
            Chips(options: CanvasBackground.allCases.map { ($0, $0.label) },
                  selection: Binding(get: { model.project.background }, set: { v in model.edit { $0.background = v } }))
            HStack(spacing: 8) {
                Button("Fill all clips") { model.edit { p in for i in p.clips.indices { p.clips[i].fill = true } } }
                Button("Fit all clips") { model.edit { p in for i in p.clips.indices { p.clips[i].fill = false } } }
            }
            .font(.caption.weight(.bold)).buttonStyle(.bordered)
            SectionLabel("Tracks")
            trackRow("Video", "film", hidden: \.mainHidden, muted: \.mainMuted, locked: \.mainLocked)
            trackRow("Overlays", "square.on.square", hidden: \.overlaysHidden, muted: \.overlaysMuted, locked: \.overlaysLocked)
            trackRow("Text", "textformat", hidden: \.textHidden, muted: nil, locked: \.textLocked)
            trackRow("Audio", "music.note", hidden: nil, muted: \.audioMuted, locked: \.audioLocked)
            Toggle("Ripple delete: titles, overlays and sounds after a deleted clip move back with it",
                   isOn: $ripple)
                .font(.caption)
            Button {
                templateName = model.project.name
                naming = true
            } label: {
                Label(savedTemplate ? "Saved as a template" : "Save as template", systemImage: savedTemplate ? "checkmark" : "rectangle.stack.badge.plus")
                    .font(.caption.weight(.bold))
            }
            .buttonStyle(.bordered)
            .disabled(model.project.clips.isEmpty)
            .alert("Save as template", isPresented: $naming) {
                TextField("Template name", text: $templateName)
                Button("Save") {
                    store.saveTemplate(from: model.project, name: templateName.isEmpty ? model.project.name : templateName)
                    savedTemplate = true
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The main clips become slots. On the home screen, pick new videos to fill them, keeping these edits, titles and music.")
            }
            SectionLabel("Project name")
            TextField("Project name", text: $name)
                .onSubmit { if !name.isEmpty { model.rename(name) } }
                .padding(10)
                .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 10))
        }
        .onAppear { name = model.project.name }
    }

    /// The project's timelines: switch, add, rename and delete.
    private var timelines: some View {
        let all = [model.project.currentTimeline] + model.project.otherTimelines
        return VStack(alignment: .leading, spacing: 8) {
            SectionLabel("Timelines")
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(all) { t in
                        let on = t.id == model.project.timelineID
                        Button { model.switchTimeline(to: t.id) } label: {
                            VStack(spacing: 2) {
                                Text(t.name).font(.caption.weight(.bold)).lineLimit(1)
                                Text("\(t.aspect.rawValue) · \(t.clips.count) clips").font(.system(size: 9))
                            }
                            .padding(.horizontal, 12).frame(height: 40)
                            .background(on ? Theme.accent : Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 10))
                            .foregroundStyle(on ? .black : .white)
                        }
                        .buttonStyle(.plain)
                    }
                    Menu {
                        Button("Copy of this timeline", systemImage: "plus.square.on.square") { model.addTimeline(copying: true) }
                        Button("Empty timeline", systemImage: "plus.square.dashed") { model.addTimeline(copying: false) }
                    } label: {
                        Image(systemName: "plus").font(.caption.weight(.bold))
                            .frame(width: 40, height: 40)
                            .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 10))
                    }
                }
            }
            HStack(spacing: 8) {
                TextField("Timeline name", text: $timelineName)
                    .onSubmit { model.renameTimeline(timelineName) }
                    .padding(8)
                    .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 8))
                if !model.project.otherTimelines.isEmpty {
                    Button(role: .destructive) { model.deleteCurrentTimeline() } label: {
                        Image(systemName: "trash").frame(width: 36, height: 34)
                    }
                    .buttonStyle(.bordered)
                }
            }
            Text("Each timeline is its own cut of this project's media: its own clips, titles, sound and canvas. Export makes the one shown.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
        }
        .onAppear { timelineName = model.project.timelineName }
        .onChange(of: model.project.timelineID) { _, _ in timelineName = model.project.timelineName }
    }

    /// One track's hide, mute and lock switches.
    private func trackRow(_ title: String, _ icon: String, hidden: WritableKeyPath<TrackFlags, Bool>?,
                          muted: WritableKeyPath<TrackFlags, Bool>?, locked: WritableKeyPath<TrackFlags, Bool>) -> some View {
        let flags = model.project.tracks
        func toggle(_ key: WritableKeyPath<TrackFlags, Bool>, on: String, off: String, _ label: String) -> some View {
            Button { model.setTracks { $0[keyPath: key].toggle() } } label: {
                Image(systemName: flags[keyPath: key] ? on : off)
                    .font(.system(size: 14, weight: .semibold))
                    .frame(width: 36, height: 32)
                    .background(flags[keyPath: key] ? Theme.accent.opacity(0.25) : Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 8))
                    .foregroundStyle(flags[keyPath: key] ? Theme.accent : .white)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
        }
        return HStack(spacing: 8) {
            Label(title, systemImage: icon).font(.subheadline)
            Spacer()
            if let hidden { toggle(hidden, on: "eye.slash.fill", off: "eye", "Hide \(title)") }
            if let muted { toggle(muted, on: "speaker.slash.fill", off: "speaker.wave.2", "Mute \(title)") }
            toggle(locked, on: "lock.fill", off: "lock.open", "Lock \(title)")
        }
    }
}
