import SwiftUI

/// Scroll-to-scrub timeline: the playhead stays centred and the tracks slide under it.
struct TimelineView: View {
    @Bindable var model: EditorModel
    var onAddMedia: () -> Void
    var onTransition: (Int) -> Void
    var onAddAudio: () -> Void

    @State private var pps: CGFloat = 60
    @State private var pinchBase: CGFloat?
    @State private var position = ScrollPosition(edge: .leading)
    @State private var phase: ScrollPhase = .idle
    @State private var thumbs = Thumbnails.shared

    private let mainHeight: CGFloat = 54
    private let overlayHeight: CGFloat = 36
    private let textHeight: CGFloat = 24
    private let audioHeight: CGFloat = 28

    var body: some View {
        GeometryReader { geo in
            let half = geo.size.width / 2
            let total = max(1, CGFloat(model.duration) * pps)
            ScrollView([.horizontal, .vertical], showsIndicators: false) {
                VStack(alignment: .leading, spacing: 5) {
                    ruler(width: total).frame(height: 18)
                    ForEach(Array(lanes(model.project.overlays, start: \.start, end: \.end).enumerated()), id: \.offset) { _, lane in
                        overlayRow(lane, width: total)
                    }
                    HStack(spacing: 10) {
                        mainRow(width: total)
                        Button(action: onAddMedia) {
                            Image(systemName: "plus").font(.system(size: 17, weight: .bold))
                                .frame(width: 42, height: mainHeight)
                                .background(Color.white, in: RoundedRectangle(cornerRadius: 8))
                                .foregroundStyle(.black)
                        }
                        .buttonStyle(.plain)
                    }
                    ForEach(Array(lanes(model.project.texts, start: \.start, end: \.end).enumerated()), id: \.offset) { _, lane in
                        textRow(lane, width: total)
                    }
                    let audioLanes = lanes(model.project.audios, start: \.start, end: { audioEnd($0) })
                    ForEach(Array(audioLanes.enumerated()), id: \.offset) { _, lane in
                        audioRow(lane, width: total)
                    }
                    if !model.project.clips.isEmpty {
                        Button(action: onAddAudio) {
                            Label(model.project.audios.isEmpty ? "Add audio" : "Audio", systemImage: "plus")
                                .font(.system(size: 11, weight: .semibold))
                                .padding(.horizontal, 10)
                                .frame(height: 24)
                                .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, half)
                .padding(.vertical, 6)
            }
            .scrollPosition($position)
            .onScrollPhaseChange { _, new in
                phase = new
                if new == .interacting { model.beginScrub() }
                if new == .idle, model.isScrubbing { model.endScrub() }
            }
            .onScrollGeometryChange(for: CGFloat.self, of: { $0.contentOffset.x }) { _, x in
                guard phase == .interacting || phase == .decelerating else { return }
                model.seek(to: Double(max(0, x) / pps))
            }
            .onChange(of: model.currentTime) { _, t in
                guard phase == .idle else { return }
                position.scrollTo(x: CGFloat(t) * pps)
            }
            .simultaneousGesture(MagnifyGesture()
                .onChanged { value in
                    if pinchBase == nil { pinchBase = pps }
                    pps = min(360, max(10, (pinchBase ?? pps) * value.magnification))
                    position.scrollTo(x: CGFloat(model.currentTime) * pps)
                }
                .onEnded { _ in pinchBase = nil })
            .overlay(alignment: .top) {
                ZStack(alignment: .top) {
                    Rectangle().fill(Color.white).frame(width: 2)
                    Circle().fill(Color.white).frame(width: 8, height: 8).offset(y: 14)
                }
                .shadow(color: .black.opacity(0.6), radius: 2)
                .allowsHitTesting(false)
            }
        }
        .background(Theme.background)
    }

    private func audioEnd(_ a: AudioItem) -> Double {
        a.loop ? max(a.start + a.length, model.duration) : a.start + a.length
    }

    /// Packs items into rows so none overlap.
    private func lanes<T>(_ items: [T], start: (T) -> Double, end: (T) -> Double) -> [[T]] {
        var rows: [[T]] = []
        var ends: [Double] = []
        for item in items.sorted(by: { start($0) < start($1) }) {
            if let i = ends.firstIndex(where: { $0 <= start(item) + 0.01 }) {
                rows[i].append(item); ends[i] = end(item)
            } else {
                rows.append([item]); ends.append(end(item))
            }
        }
        return rows
    }

    // MARK: Rows

    private func mainRow(width: CGFloat) -> some View {
        let clips = model.project.clips
        return ZStack(alignment: .topLeading) {
            Color.clear.frame(width: width, height: mainHeight)
            ForEach(Array(clips.enumerated()), id: \.element.id) { i, clip in
                let start = CGFloat(model.project.start(ofClipAt: i)) * pps
                let selected = model.selection == .clip(clip.id)
                FilmStrip(clip: clip, projectID: model.project.id, pps: pps, height: mainHeight,
                          selected: selected, accent: .white, thumbs: thumbs)
                    .offset(x: start)
                    .onTapGesture { model.selection = selected ? nil : .clip(clip.id) }
            }
            ForEach(1..<max(1, clips.count), id: \.self) { i in
                let s = model.project.start(ofClipAt: i) + model.project.overlap(at: i) / 2
                let kind = clips[i].transitionIn.kind
                Button { onTransition(i) } label: {
                    Image(systemName: kind == .none ? "plus" : kind.symbol)
                        .font(.system(size: 9, weight: .bold))
                        .frame(width: 22, height: 22)
                        .background(kind == .none ? Color.white : Theme.accent, in: RoundedRectangle(cornerRadius: 5))
                        .foregroundStyle(.black)
                        .shadow(color: .black.opacity(0.5), radius: 2)
                }
                .buttonStyle(.plain)
                .offset(x: CGFloat(s) * pps - 11, y: mainHeight / 2 - 11)
            }
        }
        .frame(width: width, height: mainHeight, alignment: .leading)
    }

    private func overlayRow(_ items: [OverlayItem], width: CGFloat) -> some View {
        ZStack(alignment: .topLeading) {
            Color.clear.frame(width: width, height: overlayHeight)
            ForEach(items) { item in
                let selected = model.selection == .overlay(item.id)
                FilmStrip(clip: item.clip, projectID: model.project.id, pps: pps, height: overlayHeight,
                          selected: selected, accent: Theme.overlayTrack, thumbs: thumbs)
                    .overlay(alignment: .bottomLeading) {
                        Image(systemName: "square.on.square").font(.system(size: 8, weight: .bold))
                            .padding(3).background(Theme.overlayTrack, in: RoundedRectangle(cornerRadius: 3)).padding(3)
                    }
                    .offset(x: CGFloat(item.start) * pps)
                    .onTapGesture { model.selection = selected ? nil : .overlay(item.id) }
            }
        }
        .frame(width: width, height: overlayHeight, alignment: .leading)
    }

    private func textRow(_ items: [TextOverlay], width: CGFloat) -> some View {
        ZStack(alignment: .topLeading) {
            Color.clear.frame(width: width, height: textHeight)
            ForEach(items) { text in
                let selected = model.selection == .text(text.id)
                let color = text.shape != nil ? Theme.shapeTrack : text.isCaption ? Theme.captionTrack : Theme.textTrack
                HStack(spacing: 4) {
                    Image(systemName: text.shape?.symbol ?? (text.isCaption ? "captions.bubble.fill" : "textformat"))
                        .font(.system(size: 9, weight: .bold))
                    Text(text.shape != nil ? text.shape!.rawValue.capitalized : text.text)
                        .font(.system(size: 10.5, weight: .semibold))
                        .lineLimit(1)
                }
                .padding(.horizontal, 6)
                .frame(width: max(16, CGFloat(text.duration) * pps - 2), height: textHeight, alignment: .leading)
                .background(color, in: RoundedRectangle(cornerRadius: 5))
                .overlay(RoundedRectangle(cornerRadius: 5).stroke(selected ? .white : .clear, lineWidth: 2))
                .offset(x: CGFloat(text.start) * pps)
                .onTapGesture { model.selection = selected ? nil : .text(text.id) }
            }
        }
        .frame(width: width, height: textHeight, alignment: .leading)
    }

    private func audioRow(_ items: [AudioItem], width: CGFloat) -> some View {
        ZStack(alignment: .topLeading) {
            Color.clear.frame(width: width, height: audioHeight)
            ForEach(items) { item in
                let selected = model.selection == .audio(item.id)
                let w = CGFloat(max(0.1, min(audioEnd(item), model.duration) - item.start)) * pps
                HStack(spacing: 5) {
                    Image(systemName: item.kind.symbol).font(.system(size: 10, weight: .bold))
                    Text(item.name).font(.system(size: 10.5, weight: .semibold)).lineLimit(1)
                    if item.effect != .none || !item.cleanup.isOff {
                        Image(systemName: "wand.and.stars").font(.system(size: 9, weight: .bold))
                    }
                    if item.loop { Image(systemName: "repeat").font(.system(size: 9, weight: .bold)) }
                }
                .padding(.horizontal, 7)
                .frame(width: max(18, w - 2), height: audioHeight, alignment: .leading)
                .background(
                    WaveformStripes()
                        .fill(Color.white.opacity(0.18))
                        .background(item.kind == .voiceover ? Theme.voiceTrack : Theme.musicTrack)
                )
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(selected ? .white : .clear, lineWidth: 2))
                .offset(x: CGFloat(item.start) * pps)
                .onTapGesture { model.selection = selected ? nil : .audio(item.id) }
            }
        }
        .frame(width: width, height: audioHeight, alignment: .leading)
    }

    private func ruler(width: CGFloat) -> some View {
        // A label every 1, 2, 5 or 10 seconds depending on zoom.
        let every: Double = pps > 90 ? 1 : pps > 40 ? 2 : pps > 18 ? 5 : 10
        return Canvas { ctx, size in
            var t = 0.0
            while CGFloat(t) * pps <= size.width + 1 {
                let x = CGFloat(t) * pps
                let major = t.truncatingRemainder(dividingBy: every) == 0
                ctx.fill(Path(CGRect(x: x, y: major ? 10 : 13, width: 1, height: major ? 8 : 5)),
                         with: .color(.white.opacity(major ? 0.45 : 0.2)))
                if major {
                    ctx.draw(Text(t.timecode.replacingOccurrences(of: ".0", with: "")).font(.system(size: 9)).foregroundStyle(.secondary),
                             at: CGPoint(x: x + 2, y: 4), anchor: .leading)
                }
                t += every >= 5 ? 1 : 0.5
            }
        }
        .frame(width: max(width, 1))
    }
}

/// A clip as a strip of frames, with badges for what's been done to it.
struct FilmStrip: View {
    let clip: Clip
    let projectID: UUID
    let pps: CGFloat
    let height: CGFloat
    let selected: Bool
    let accent: Color
    let thumbs: Thumbnails

    var body: some View {
        let width = max(8, CGFloat(clip.duration) * pps)
        let tile = height * 0.8
        let count = max(1, Int(ceil(width / tile)))
        _ = thumbs.strip(for: clip, projectID: projectID)
        return HStack(spacing: 0) {
            ForEach(0..<count, id: \.self) { k in
                let source = clip.sourceTime(atLocal: Double(CGFloat(k) * tile / pps))
                Group {
                    if let image = thumbs.frame(clip.file, at: source) {
                        Image(uiImage: image).resizable().scaledToFill()
                    } else {
                        Color.white.opacity(0.08)
                    }
                }
                .frame(width: tile, height: height)
                .clipped()
            }
        }
        .frame(width: width, height: height, alignment: .leading)
        .clipped()
        .overlay(alignment: .topLeading) {
            HStack(spacing: 3) {
                if clip.speedCurve != .none { badge(clip.speedCurve.label, "speedometer") }
                else if clip.speed != 1 { badge(String(format: "%.2gx", clip.speed), nil) }
                if clip.filter != "none" { badge(LookCatalog.name(clip.filter), nil) }
                if !clip.effects.isEmpty { badge("\(clip.effects.count)", "sparkles") }
                if clip.cutout != .none { badge(nil, "person.crop.rectangle") }
                if clip.volume == 0 { badge(nil, "speaker.slash.fill") }
                if clip.audioEffect != .none || !clip.cleanup.isOff { badge(nil, "waveform") }
            }
            .padding(3)
        }
        .overlay(alignment: .bottomLeading) {
            ZStack(alignment: .leading) {
                ForEach(clip.keyframes) { k in
                    Image(systemName: "diamond.fill")
                        .font(.system(size: 8))
                        .foregroundStyle(Theme.accent)
                        .shadow(color: .black, radius: 1)
                        .offset(x: CGFloat(k.time) * pps - 4)
                }
            }
            .padding(.bottom, 3)
        }
        .clipShape(RoundedRectangle(cornerRadius: 7))
        .overlay(RoundedRectangle(cornerRadius: 7).stroke(selected ? accent : .clear, lineWidth: 2.5))
        .contentShape(Rectangle())
    }

    private func badge(_ s: String?, _ icon: String?) -> some View {
        HStack(spacing: 2) {
            if let icon { Image(systemName: icon) }
            if let s { Text(s) }
        }
        .font(.system(size: 8.5, weight: .bold))
        .padding(.horizontal, 4).padding(.vertical, 1.5)
        .background(.black.opacity(0.62), in: Capsule())
        .lineLimit(1)
    }
}

/// Decorative bars suggesting a waveform.
struct WaveformStripes: Shape {
    func path(in rect: CGRect) -> Path {
        var p = Path()
        var x: CGFloat = 2
        var i = 0
        while x < rect.width {
            let h = rect.height * (0.25 + 0.55 * abs(sin(Double(i) * 0.9) * cos(Double(i) * 0.37)))
            p.addRoundedRect(in: CGRect(x: x, y: (rect.height - h) / 2, width: 2, height: h), cornerSize: CGSize(width: 1, height: 1))
            x += 4; i += 1
        }
        return p
    }
}
