import SwiftUI

enum ToolMenu { case root, audio, text }

/// The bottom row of tools: top-level categories, a sub-menu, or the tools for what's selected.
struct ToolBar: View {
    @Bindable var model: EditorModel
    @Binding var menu: ToolMenu
    var open: (Panel) -> Void
    var addMedia: (PickerTarget) -> Void

    var body: some View {
        HStack(spacing: 0) {
            if model.selection != nil || menu != .root {
                Button {
                    if model.selection != nil { model.selection = nil } else { menu = .root }
                } label: {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 15, weight: .bold))
                        .frame(width: 40, height: 56)
                        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 12))
                }
                .buttonStyle(.plain)
                .padding(.leading, 10)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 2) { tools }.padding(.horizontal, 8)
            }
        }
        .foregroundStyle(.white)
        .frame(maxHeight: .infinity)
    }

    /// Opens a clip panel, picking the clip under the playhead if none is chosen.
    private func clipTool(_ panel: Panel) {
        if model.selectClipAtPlayhead() { open(panel) }
    }

    @ViewBuilder
    private var tools: some View {
        switch model.selection {
        case .clip:
            Tool("Split", "scissors") { model.splitAtPlayhead() }
            Tool("Trim", "timeline.selection") { open(.trim) }
            Tool("Speed", "speedometer") { open(.speed) }
            Tool("Animate", "diamond.lefthalf.filled") { open(.animate) }
            Tool("Filters", "camera.filters") { open(.filters) }
            Tool("Effects", "sparkles") { open(.effects) }
            Tool("Adjust", "slider.horizontal.3") { open(.adjust) }
            Tool("Remove BG", "person.crop.rectangle") { open(.cutout) }
            Tool("Brush", "paintbrush.pointed.fill") { open(.brush) }
            Tool("Stabilize", "camera.metering.matrix") { open(.stabilize) }
            Tool("Crop", "crop") { open(.crop) }
            Tool("Volume", "speaker.wave.2.fill") { open(.volume) }
            Tool("Voice FX", "waveform.badge.magnifyingglass") { open(.voiceFX) }
            Tool("Extract audio", "waveform.and.arrow.down") { model.extractAudio() }
            if let i = model.selectedClipIndex, i > 0 {
                Tool("Transition", "square.split.diagonal.2x2") { open(.transition(i)) }
            }
            Tool("Fade", "circle.lefthalf.filled") { open(.fade) }
            Tool("Freeze", "snowflake") { Task { await model.freezeFrame() } }
            Tool(model.selectedMedia?.fill == true ? "Fit" : "Fill", "arrow.up.left.and.arrow.down.right") {
                model.mediaBinding(\.fill, fallback: false).wrappedValue.toggle()
            }
            Tool("Mirror", "arrow.left.and.right.righttriangle.left.righttriangle.right") {
                model.mediaBinding(\.flipped, fallback: false).wrappedValue.toggle()
            }
            Tool("Flip", "arrow.up.and.down.righttriangle.up.righttriangle.down") {
                model.mediaBinding(\.flippedV, fallback: false).wrappedValue.toggle()
            }
            if let i = model.selectedClipIndex, i + 1 < model.project.clips.count {
                Tool("Merge", "arrow.triangle.merge") { model.mergeWithNext() }
            }
            if let id = model.selectedClipIndex.map({ model.project.clips[$0].id }) {
                Tool("Replace", "arrow.triangle.2.circlepath") { addMedia(.replace(id)) }
            }
            Tool("Move left", "arrow.left.to.line") { model.moveSelectedClip(by: -1) }
            Tool("Move right", "arrow.right.to.line") { model.moveSelectedClip(by: 1) }
            Tool("Duplicate", "plus.square.on.square") { model.duplicateSelection() }
            Tool("Delete", "trash", destructive: true) { model.deleteSelection() }
        case .overlay:
            Tool("Split", "scissors") { model.splitAtPlayhead() }
            Tool("Animate", "diamond.lefthalf.filled") { open(.animate) }
            Tool("Blend", "square.on.square.intersection.dashed") { open(.blend) }
            Tool("Mask", "circle.dashed.inset.filled") { open(.mask) }
            Tool("Remove BG", "person.crop.rectangle") { open(.cutout) }
            Tool("Brush", "paintbrush.pointed.fill") { open(.brush) }
            Tool("Stabilize", "camera.metering.matrix") { open(.stabilize) }
            Tool("Filters", "camera.filters") { open(.filters) }
            Tool("Effects", "sparkles") { open(.effects) }
            Tool("Adjust", "slider.horizontal.3") { open(.adjust) }
            Tool("Crop", "crop") { open(.crop) }
            Tool("Trim", "timeline.selection") { open(.trim) }
            Tool("Speed", "speedometer") { open(.speed) }
            Tool("Volume", "speaker.wave.2.fill") { open(.volume) }
            Tool("Voice FX", "waveform.badge.magnifyingglass") { open(.voiceFX) }
            Tool("Fade", "circle.lefthalf.filled") { open(.fade) }
            Tool("Mirror", "arrow.left.and.right.righttriangle.left.righttriangle.right") {
                model.mediaBinding(\.flipped, fallback: false).wrappedValue.toggle()
            }
            if let id = model.selectedOverlayIndex.map({ model.project.overlays[$0].id }) {
                Tool("Replace", "arrow.triangle.2.circlepath") { addMedia(.replace(id)) }
            }
            Tool("Duplicate", "plus.square.on.square") { model.duplicateSelection() }
            Tool("Delete", "trash", destructive: true) { model.deleteSelection() }
        case .text:
            let isShape = model.selectedTextIndex.map { model.project.texts[$0].shape != nil } ?? false
            if !isShape { Tool("Edit", "character.cursor.ibeam") { open(.text(.edit)) } }
            Tool("Style", "paintbrush.pointed.fill") { open(.text(.style)) }
            if !isShape { Tool("Font", "textformat") { open(.text(.font)) } }
            Tool("Animate", "wand.and.stars") { open(.text(.animate)) }
            Tool("Split", "scissors") { model.splitAtPlayhead() }
            Tool("Duplicate", "plus.square.on.square") { model.duplicateSelection() }
            Tool("Delete", "trash", destructive: true) { model.deleteSelection() }
        case .audio:
            Tool("Volume", "speaker.wave.2.fill") { open(.audioItem) }
            Tool("Voice FX", "waveform.badge.magnifyingglass") { open(.voiceFX) }
            if model.selectedAudioIndex.flatMap({ model.project.audios[$0].sourceClip }) != nil {
                Tool("Reattach", "arrow.uturn.left.circle") { model.reattachAudio() }
            }
            Tool("Split", "scissors") { model.splitAtPlayhead() }
            Tool("Duplicate", "plus.square.on.square") { model.duplicateSelection() }
            Tool("Delete", "trash", destructive: true) { model.deleteSelection() }
        case nil:
            switch menu {
            case .root:
                Tool("Edit", "scissors") { model.selectClipAtPlayhead() }
                Tool("Audio", "music.note") { menu = .audio }
                Tool("Text", "textformat") { menu = .text }
                Tool("Overlay", "square.on.square") { addMedia(.overlay) }
                Tool("Effects", "sparkles") { clipTool(.effects) }
                Tool("Filters", "camera.filters") { clipTool(.filters) }
                Tool("Adjust", "slider.horizontal.3") { clipTool(.adjust) }
                Tool("Captions", "captions.bubble") { open(.captions) }
                Tool("Remove BG", "person.crop.rectangle") { clipTool(.cutout) }
                Tool("Transitions", "square.split.diagonal.2x2") {
                    let i = max(1, model.project.clip(at: model.currentTime)?.index ?? 1)
                    if model.project.clips.count > 1 { open(.transition(min(i, model.project.clips.count - 1))) }
                }
                Tool("Canvas", "aspectratio") { open(.canvas) }
                Tool("Add media", "plus.rectangle.on.rectangle") { addMedia(.mainTrack) }
            case .audio:
                Tool("Add audio", "music.note.list") { open(.music) }
                Tool("Record", "mic.fill") { open(.record) }
                Tool("Text to speech", "text.bubble.fill") { open(.tts) }
                Tool("Extract audio", "waveform.and.arrow.down") { model.extractAudio() }
            case .text:
                Tool("Add text", "plus") { model.addText(); open(.text(.edit)) }
                Tool("Templates", "textformat.alt") { open(.textTemplates) }
                Tool("Captions", "captions.bubble") { open(.captions) }
                Tool("Text to speech", "text.bubble.fill") { open(.tts) }
                Tool("Shapes", "star.square.on.square") { open(.shapes) }
            }
        }
    }
}

struct Tool: View {
    let title: String
    let icon: String
    var destructive = false
    let action: () -> Void

    init(_ title: String, _ icon: String, destructive: Bool = false, action: @escaping () -> Void) {
        self.title = title; self.icon = icon; self.destructive = destructive; self.action = action
    }

    var body: some View {
        Button(action: action) {
            VStack(spacing: 7) {
                Image(systemName: icon)
                    .font(.system(size: 19, weight: .medium))
                    .frame(height: 24)
                Text(LocalizedStringKey(title))
                    .font(.system(size: 10.5, weight: .medium))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .frame(width: 64, height: 60)
            .foregroundStyle(destructive ? Color(red: 1, green: 0.38, blue: 0.38) : .white)
            .contentShape(Rectangle())
        }
        .buttonStyle(PressStyle())
    }
}

struct PressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.55 : 1)
            .scaleEffect(configuration.isPressed ? 0.94 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}
