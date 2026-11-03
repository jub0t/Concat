import AVFoundation
import PhotosUI
import SwiftUI

/// Creates the editor model once per presentation.
struct EditorHost: View {
    let project: Project
    @Environment(ProjectStore.self) private var store
    @State private var model: EditorModel?

    var body: some View {
        Group {
            if let model { EditorView(model: model) } else { Theme.background.ignoresSafeArea() }
        }
        .task { if model == nil { model = EditorModel(project: project, store: store) } }
    }
}

enum Panel: Hashable {
    case speed, volume, animate, crop, filters, effects, adjust, cutout, voiceFX, fade, trim
    case transition(Int), blend, mask
    case text(TextTab), textTemplates, shapes, captions
    case tts, music, record, audioItem
    case canvas
}

enum TextTab: String, CaseIterable, Hashable {
    case edit = "Edit", style = "Style", font = "Font", animate = "Animate"
}

enum PickerTarget { case mainTrack, overlay }

struct EditorView: View {
    @Bindable var model: EditorModel
    @Environment(\.dismiss) private var dismiss
    @State private var panel: Panel?
    @State private var menu: ToolMenu = .root
    @State private var showExport = false
    @State private var picker: PickerTarget?
    @State private var pickerItems: [PhotosPickerItem] = []
    @State private var importing = false

    var body: some View {
        GeometryReader { geo in
            let total = geo.size.height
            let topH: CGFloat = 50
            let transportH: CGFloat = 44
            let bottomH: CGFloat = panel == nil ? 86 : min(330, max(250, total * 0.36))
            let timelineH: CGFloat = max(150, min(232, total * 0.27))
            let previewH = max(120, total - topH - transportH - bottomH - timelineH)
            VStack(spacing: 0) {
                topBar.frame(height: topH)
                PreviewArea(model: model, size: CGSize(width: geo.size.width - 24, height: previewH - 12))
                    .frame(width: geo.size.width, height: previewH)
                transport.frame(height: transportH)
                TimelineView(model: model,
                             onAddMedia: { picker = .mainTrack },
                             onTransition: { index in model.selection = nil; open(.transition(index)) },
                             onAddAudio: { menu = .audio })
                    .frame(height: timelineH)
                bottomArea
                    .frame(height: bottomH)
                    .animation(.snappy(duration: 0.22), value: panel)
            }
        }
        .background(Theme.background.ignoresSafeArea())
        .preferredColorScheme(.dark)
        .onChange(of: model.selection) { old, new in
            if old != new, !(panel.map(Self.keepsPanel) ?? false) { panel = nil }
            if new != nil { menu = .root }
        }
        .photosPicker(isPresented: Binding(get: { picker != nil }, set: { if !$0 && pickerItems.isEmpty { picker = nil } }),
                      selection: $pickerItems, maxSelectionCount: 30,
                      matching: .any(of: [.videos, .images]), preferredItemEncoding: .current)
        .onChange(of: pickerItems) { _, items in
            guard !items.isEmpty else { return }
            let target = picker ?? .mainTrack
            Task {
                importing = true
                let (clips, failures) = await PickerImport.clips(from: items, projectID: model.project.id)
                if target == .overlay { model.addOverlays(clips) } else { model.addClips(clips) }
                if !failures.isEmpty { model.errorMessage = failures.joined(separator: "\n") }
                pickerItems = []
                picker = nil
                importing = false
            }
        }
        .sheet(isPresented: $showExport) { ExportSheet(project: model.project) }
        .alert("Something went wrong", isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.errorMessage = nil } })) {
            Button("OK") {}
        } message: { Text(model.errorMessage ?? "") }
        .overlay {
            if importing || model.busyMessage != nil {
                VStack(spacing: 12) {
                    ProgressView().controlSize(.large)
                    Text(model.busyMessage ?? "Importing…").font(.subheadline.weight(.medium))
                }
                .padding(28)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 20))
            }
        }
    }

    /// Panels that create the selection they then edit stay open when it changes.
    private static func keepsPanel(_ p: Panel) -> Bool {
        switch p {
        case .text, .captions, .tts, .music, .record, .textTemplates, .shapes, .canvas: true
        default: false
        }
    }

    private func open(_ p: Panel) {
        withAnimation(.snappy(duration: 0.22)) { panel = p }
    }

    // MARK: Top and transport

    private var topBar: some View {
        HStack(spacing: 10) {
            Button {
                model.close()
                dismiss()
            } label: {
                Image(systemName: "chevron.down")
                    .font(.system(size: 16, weight: .bold))
                    .frame(width: 38, height: 38)
                    .background(Theme.surface, in: Circle())
            }
            Text(model.project.name)
                .font(.subheadline.weight(.semibold))
                .lineLimit(1)
                .foregroundStyle(Theme.secondaryText)
            Spacer(minLength: 4)
            Button { open(.canvas) } label: {
                HStack(spacing: 5) {
                    Image(systemName: "aspectratio").font(.caption.weight(.bold))
                    Text(model.project.aspect.rawValue).font(.caption.weight(.bold))
                }
                .padding(.horizontal, 11).frame(height: 32)
                .background(Theme.surface, in: Capsule())
            }
            Button {
                model.player.pause(); model.isPlaying = false
                showExport = true
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "square.and.arrow.up").font(.caption.weight(.bold))
                    Text("Export").font(.subheadline.weight(.bold))
                }
                .padding(.horizontal, 14).frame(height: 34)
                .background(Theme.accent, in: Capsule())
                .foregroundStyle(.black)
            }
            .disabled(model.project.clips.isEmpty)
            .opacity(model.project.clips.isEmpty ? 0.4 : 1)
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
        .padding(.horizontal, 14)
    }

    private var transport: some View {
        HStack(spacing: 0) {
            HStack(spacing: 3) {
                Text(model.currentTime.timecode).foregroundStyle(.white)
                Text("/ \(model.duration.timecode)").foregroundStyle(Theme.secondaryText)
            }
            .font(.system(size: 12, weight: .medium).monospacedDigit())
            .frame(width: 118, alignment: .leading)
            Spacer()
            Button { model.togglePlay() } label: {
                Image(systemName: model.isPlaying ? "pause.fill" : "play.fill")
                    .font(.system(size: 20))
                    .frame(width: 46, height: 40)
            }
            .disabled(model.project.clips.isEmpty)
            Spacer()
            HStack(spacing: 4) {
                if model.selectedMedia != nil {
                    Button { model.toggleKeyframe() } label: {
                        Image(systemName: model.keyframeAtPlayhead ? "diamond.fill" : "diamond")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(model.keyframeAtPlayhead ? Theme.accent : .white)
                            .frame(width: 36, height: 36)
                    }
                    .accessibilityLabel("Keyframe")
                }
                Button { model.undo() } label: { Image(systemName: "arrow.uturn.backward").frame(width: 36, height: 36) }
                    .disabled(!model.canUndo).opacity(model.canUndo ? 1 : 0.35)
                Button { model.redo() } label: { Image(systemName: "arrow.uturn.forward").frame(width: 36, height: 36) }
                    .disabled(!model.canRedo).opacity(model.canRedo ? 1 : 0.35)
            }
            .font(.system(size: 15, weight: .semibold))
            .frame(width: 118, alignment: .trailing)
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
        .padding(.horizontal, 14)
        .overlay(alignment: .center) {
            if model.isBuilding { ProgressView().controlSize(.mini).offset(x: 38) }
        }
    }

    // MARK: Bottom

    @ViewBuilder
    private var bottomArea: some View {
        if let panel {
            VStack(spacing: 0) {
                HStack {
                    Text(title(for: panel)).font(.subheadline.weight(.bold))
                    Spacer()
                    Button {
                        withAnimation(.snappy(duration: 0.22)) { self.panel = nil }
                    } label: {
                        Image(systemName: "checkmark")
                            .font(.system(size: 15, weight: .bold))
                            .frame(width: 34, height: 30)
                            .background(Theme.accent, in: Capsule())
                            .foregroundStyle(.black)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 16).padding(.top, 12).padding(.bottom, 6)
                ScrollView {
                    PanelContent(panel: panel, model: model, open: { open($0) })
                        .padding(.horizontal, 16).padding(.bottom, 16)
                }
                .scrollDismissesKeyboard(.interactively)
            }
            .background(Theme.surface)
            .clipShape(UnevenRoundedRectangle(topLeadingRadius: 20, topTrailingRadius: 20))
        } else {
            ToolBar(model: model, menu: $menu, open: { open($0) }, addMedia: { picker = $0 })
                .background(Theme.background)
                .overlay(alignment: .top) { Rectangle().fill(Color.white.opacity(0.06)).frame(height: 1) }
        }
    }

    private func title(for panel: Panel) -> String {
        switch panel {
        case .speed: "Speed"
        case .volume: "Volume"
        case .animate: "Animate"
        case .crop: "Crop"
        case .filters: "Filters"
        case .effects: "Effects"
        case .adjust: "Adjust"
        case .cutout: "Remove background"
        case .voiceFX: "Voice effects"
        case .fade: "Fade"
        case .trim: "Trim"
        case .transition: "Transition"
        case .blend: "Blend"
        case .mask: "Mask"
        case .text: "Text"
        case .textTemplates: "Text templates"
        case .shapes: "Shapes"
        case .captions: "Auto captions"
        case .tts: "Text to speech"
        case .music: "Add audio"
        case .record: "Record voiceover"
        case .audioItem: "Audio"
        case .canvas: "Canvas"
        }
    }
}

// MARK: Preview

/// The finished picture, plus direct manipulation of whatever is selected.
struct PreviewArea: View {
    @Bindable var model: EditorModel
    let size: CGSize
    @State private var dragStart: Transform?
    @State private var textStart: CGPoint?
    @State private var pinchStart: Double?
    @State private var rotateStart: Double?

    var body: some View {
        let ratio = model.project.aspect.ratio
        let w = max(10, min(size.width, size.height * ratio))
        let h = w / ratio
        ZStack {
            FrameView(model: model)
            if model.project.clips.isEmpty {
                VStack(spacing: 10) {
                    Image(systemName: "film.stack").font(.system(size: 34))
                    Text("Add media to start").font(.subheadline)
                }
                .foregroundStyle(Theme.secondaryText)
            }
            manipulation(CGSize(width: w, height: h))
        }
        .frame(width: w, height: h)
        .background(Color.black)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Color.white.opacity(0.08)))
    }

    @ViewBuilder
    private func manipulation(_ box: CGSize) -> some View {
        ZStack {
            Color.white.opacity(0.001)
                .contentShape(Rectangle())
                .onTapGesture { model.togglePlay() }
                .gesture(mediaDrag(box))
                .simultaneousGesture(mediaPinch)
                .simultaneousGesture(mediaRotate)
            ForEach(visibleTexts) { text in
                let b = estimatedBox(text, in: box)
                let selected = model.selection == .text(text.id)
                Rectangle()
                    .fill(Color.white.opacity(0.001))
                    .overlay {
                        if selected {
                            RoundedRectangle(cornerRadius: 4).stroke(Color.white, style: StrokeStyle(lineWidth: 1.5, dash: [5, 3]))
                        }
                    }
                    .frame(width: b.width, height: b.height)
                    .rotationEffect(.degrees(text.rotation))
                    .position(x: text.x * box.width, y: text.y * box.height)
                    .onTapGesture { model.selection = .text(text.id) }
                    .gesture(DragGesture(minimumDistance: 2)
                        .onChanged { value in
                            if textStart == nil {
                                model.selection = .text(text.id)
                                textStart = CGPoint(x: text.x, y: text.y)
                            }
                            guard let s = textStart else { return }
                            model.textBinding(\.x, fallback: 0.5).wrappedValue = min(1, max(0, s.x + value.translation.width / box.width))
                            model.textBinding(\.y, fallback: 0.5).wrappedValue = min(1, max(0, s.y + value.translation.height / box.height))
                        }
                        .onEnded { _ in textStart = nil })
            }
        }
    }

    private func mediaDrag(_ box: CGSize) -> some Gesture {
        DragGesture(minimumDistance: 4)
            .onChanged { value in
                guard let local = model.selectedLocalTime, let clip = model.selectedMedia else { return }
                if dragStart == nil { dragStart = clip.transform(atLocal: local) }
                guard let s = dragStart else { return }
                model.transformBinding(\.x).wrappedValue = s.x + value.translation.width / box.width
                model.transformBinding(\.y).wrappedValue = s.y + value.translation.height / box.height
            }
            .onEnded { _ in dragStart = nil }
    }

    private var mediaPinch: some Gesture {
        MagnifyGesture()
            .onChanged { value in
                guard model.selectedMedia != nil else { return }
                if pinchStart == nil { pinchStart = model.transformBinding(\.scale).wrappedValue }
                model.transformBinding(\.scale).wrappedValue = max(0.05, min(8, (pinchStart ?? 1) * value.magnification))
            }
            .onEnded { _ in pinchStart = nil }
    }

    private var mediaRotate: some Gesture {
        RotateGesture()
            .onChanged { value in
                guard model.selectedMedia != nil else { return }
                if rotateStart == nil { rotateStart = model.transformBinding(\.rotation).wrappedValue }
                model.transformBinding(\.rotation).wrappedValue = (rotateStart ?? 0) + value.rotation.degrees
            }
            .onEnded { _ in rotateStart = nil }
    }

    private var visibleTexts: [TextOverlay] {
        model.project.texts.filter { model.currentTime >= $0.start && model.currentTime < $0.end }
    }

    /// Roughly where the rendered title sits, for hit testing.
    private func estimatedBox(_ text: TextOverlay, in size: CGSize) -> CGSize {
        if text.shape != nil {
            let side = text.size * size.width * 4
            return CGSize(width: max(40, side), height: max(40, side))
        }
        let font = text.size * size.width
        let lines = max(1, text.text.split(separator: "\n").count)
        let longest = text.text.split(separator: "\n").map(\.count).max() ?? text.text.count
        let width = min(size.width * 0.92, CGFloat(longest) * font * 0.6 + font)
        return CGSize(width: max(44, width), height: max(32, CGFloat(lines) * font * 1.35 + font * 0.4))
    }
}

/// Shows the edit. Paused, it asks the composition for the exact frame; playing,
/// it shows the composed frames the player produces.
struct FrameView: UIViewRepresentable {
    let model: EditorModel

    final class DisplayView: UIView {
        weak var model: EditorModel?
        private var link: CADisplayLink?
        private let context = CIContext(options: [.cacheIntermediates: false])
        private var shownTime = -1.0
        private var drawnToken = -1
        private var fetching = false

        override func didMoveToWindow() {
            super.didMoveToWindow()
            link?.invalidate()
            guard window != nil else { return }
            link = CADisplayLink(target: self, selector: #selector(tick))
            link?.add(to: .main, forMode: .common)
        }

        private var targetSide: CGFloat { max(bounds.width, bounds.height) * (window?.screen.scale ?? 3) }

        @objc private func tick() {
            guard let model else { return }
            if model.isPlaying, let output = model.videoOutput {
                let time = output.itemTime(forHostTime: CACurrentMediaTime())
                guard output.hasNewPixelBuffer(forItemTime: time),
                      let buffer = output.copyPixelBuffer(forItemTime: time, itemTimeForDisplay: nil) else { return }
                var image = CIImage(cvPixelBuffer: buffer)
                if !model.livePipeline {
                    let track = model.mainTrack(at: time.seconds) ?? 1
                    image = FrameRenderer.render(sources: [track: image], time: time.seconds, state: model.renderState)
                }
                show(image)
                shownTime = time.seconds
                drawnToken = model.redrawToken
                return
            }
            let needs = abs(model.currentTime - shownTime) > 0.01 || drawnToken != model.redrawToken
            guard needs, !fetching, model.built != nil else { return }
            fetching = true
            let t = model.currentTime
            let token = model.redrawToken
            let side = targetSide
            Task { @MainActor in
                if let cg = await model.stillFrame(at: t, maxSide: side) {
                    layer.contents = cg
                    shownTime = t
                    drawnToken = token
                }
                fetching = false
            }
        }

        private func show(_ image: CIImage) {
            var img = image
            let side = targetSide
            let longest = max(img.extent.width, img.extent.height)
            if longest > side, side > 0 {
                let s = side / longest
                img = img.transformed(by: CGAffineTransform(scaleX: s, y: s))
            }
            if let cg = context.createCGImage(img, from: img.extent) { layer.contents = cg }
        }
    }

    func makeUIView(context: Context) -> DisplayView {
        let view = DisplayView()
        view.model = model
        view.backgroundColor = .black
        view.layer.contentsGravity = .resizeAspect
        return view
    }

    func updateUIView(_ view: DisplayView, context: Context) { view.model = model }
}
