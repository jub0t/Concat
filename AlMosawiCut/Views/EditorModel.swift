import AVFoundation
import Observation
import SwiftUI

enum Selection: Equatable {
    case clip(UUID), overlay(UUID), text(UUID), audio(UUID)
}

/// The open project: edits with undo, and the player that previews them.
@MainActor
@Observable
final class EditorModel {
    private(set) var project: Project
    var selection: Selection?
    var currentTime: Double = 0
    var isPlaying = false
    var isBuilding = false
    var busyMessage: String?
    var errorMessage: String?
    /// True while the timeline is being dragged, so playback doesn't fight it.
    var isScrubbing = false
    var canUndo = false
    var canRedo = false

    let player = AVPlayer()
    @ObservationIgnored private let store: ProjectStore
    @ObservationIgnored private let state: RenderState
    @ObservationIgnored private(set) var built: BuiltComposition?
    @ObservationIgnored private var builtKey = ""
    @ObservationIgnored private var rebuildTask: Task<Void, Never>?
    @ObservationIgnored private var saveTask: Task<Void, Never>?
    @ObservationIgnored private var timeObserver: Any?
    @ObservationIgnored private var statusObservation: NSKeyValueObservation?
    @ObservationIgnored private var undoStack: [Project] = []
    @ObservationIgnored private var redoStack: [Project] = []
    @ObservationIgnored private var lastUndoPush = Date.distantPast

    /// Live playback draws through the video composition. The Simulator can't
    /// play compositions, so there the preview finishes raw frames itself.
    @ObservationIgnored private(set) var livePipeline: Bool = {
        #if targetEnvironment(simulator)
        false
        #else
        true
        #endif
    }()
    @ObservationIgnored private(set) var videoOutput: AVPlayerItemVideoOutput?
    /// Bumped whenever the drawn result may differ, so a paused preview redraws.
    @ObservationIgnored private(set) var redrawToken = 0
    var renderState: RenderState { state }

    init(project: Project, store: ProjectStore) {
        self.project = project
        self.store = store
        state = RenderState(project: project, segments: [], canvas: project.aspect.baseSize)
        player.actionAtItemEnd = .pause
        timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 30), queue: .main) { [weak self] time in
            MainActor.assumeIsolated {
                guard let self, !self.isScrubbing else { return }
                self.currentTime = time.seconds
                if self.isPlaying, self.player.rate == 0 { self.isPlaying = false }
            }
        }
        sync()
    }

    func close() {
        player.pause()
        if let timeObserver { player.removeTimeObserver(timeObserver) }
        saveTask?.cancel()
        store.save(project)
    }

    var duration: Double { project.duration }

    // MARK: Editing

    /// Every change goes through here. Rapid changes (a slider drag) share one undo step.
    func edit(_ change: (inout Project) -> Void) {
        let before = project
        change(&project)
        guard project != before else { return }
        if Date().timeIntervalSince(lastUndoPush) > 0.7 {
            undoStack.append(before)
            if undoStack.count > 100 { undoStack.removeFirst() }
            redoStack.removeAll()
        }
        lastUndoPush = Date()
        changed()
    }

    func undo() {
        guard let previous = undoStack.popLast() else { return }
        redoStack.append(project)
        project = previous
        lastUndoPush = .distantPast
        changed()
    }

    func redo() {
        guard let next = redoStack.popLast() else { return }
        undoStack.append(project)
        project = next
        lastUndoPush = .distantPast
        changed()
    }

    private func changed() {
        canUndo = !undoStack.isEmpty
        canRedo = !redoStack.isEmpty
        switch selection {
        case .clip(let id) where !project.clips.contains(where: { $0.id == id }): selection = nil
        case .overlay(let id) where !project.overlays.contains(where: { $0.id == id }): selection = nil
        case .text(let id) where !project.texts.contains(where: { $0.id == id }): selection = nil
        case .audio(let id) where !project.audios.contains(where: { $0.id == id }): selection = nil
        default: break
        }
        sync()
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 800_000_000)
            guard let self, !Task.isCancelled else { return }
            self.store.save(self.project)
        }
    }

    // MARK: Preview

    /// Structural changes rebuild the composition; looks only redraw, because
    /// the compositor reads the render state for every frame.
    private func sync() {
        state.update(project: project, canvas: project.aspect.baseSize)
        redrawToken &+= 1
        if project.structureKey != builtKey { scheduleRebuild() }
    }

    private func scheduleRebuild() {
        rebuildTask?.cancel()
        let snapshot = project
        rebuildTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 120_000_000)
            guard let self, !Task.isCancelled else { return }
            self.isBuilding = true
            defer { self.isBuilding = false }
            do {
                guard !snapshot.clips.isEmpty else {
                    self.built = nil
                    self.builtKey = snapshot.structureKey
                    self.player.replaceCurrentItem(with: nil)
                    return
                }
                let built = try await CompositionBuilder.build(snapshot)
                guard !Task.isCancelled else { return }
                self.install(built, key: snapshot.structureKey, duration: snapshot.duration)
            } catch {
                self.errorMessage = error.localizedDescription
            }
        }
    }

    private func install(_ built: BuiltComposition, key: String, duration: Double) {
        self.built = built
        builtKey = key
        state.update(project: project, segments: built.segments)
        let item = AVPlayerItem(asset: built.composition)
        let output = AVPlayerItemVideoOutput(pixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA])
        item.add(output)
        videoOutput = output
        item.audioMix = built.audioMix
        if livePipeline { item.videoComposition = built.videoComposition(state: state) }
        statusObservation = item.observe(\.status) { [weak self] item, _ in
            guard item.status == .failed else { return }
            Task { @MainActor in
                guard let self else { return }
                if self.livePipeline {
                    // Fall back to drawing frames ourselves rather than showing nothing.
                    self.livePipeline = false
                    self.builtKey = ""
                    self.sync()
                } else {
                    self.errorMessage = item.error?.localizedDescription ?? "The preview couldn't load."
                }
            }
        }
        let resume = min(currentTime, max(0, duration - 0.01))
        let wasPlaying = isPlaying
        player.replaceCurrentItem(with: item)
        player.seek(to: CMTime(seconds: resume, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
            Task { @MainActor in
                if wasPlaying { self?.player.play() }
                self?.redrawToken &+= 1
            }
        }
    }

    /// The finished frame at `t`, for a paused or scrubbed preview.
    func stillFrame(at t: Double, maxSide: CGFloat) async -> CGImage? {
        guard let built else { return nil }
        let generator = AVAssetImageGenerator(asset: built.composition)
        generator.videoComposition = built.videoComposition(state: state)
        generator.maximumSize = CGSize(width: maxSide, height: maxSide)
        let tolerance = isScrubbing ? CMTime(value: 1, timescale: 30) : .zero
        generator.requestedTimeToleranceBefore = tolerance
        generator.requestedTimeToleranceAfter = tolerance
        let time = CMTime(seconds: min(t, max(0, duration - 0.02)), preferredTimescale: 600)
        return try? await generator.image(at: time).image
    }

    /// The main-track segment showing at `t`, for the Simulator's raw-frame preview.
    func mainTrack(at t: Double) -> CMPersistentTrackID? {
        state.snapshot.1.last { !$0.isOverlay && t >= $0.start && t < $0.end }?.trackID
    }

    func togglePlay() {
        if isPlaying {
            player.pause()
            isPlaying = false
        } else {
            if currentTime >= duration - 0.05 { seek(to: 0) }
            player.play()
            isPlaying = true
        }
    }

    func seek(to t: Double) {
        let t = max(0, min(t, duration))
        currentTime = t
        let tolerance = isScrubbing ? CMTime(value: 1, timescale: 30) : .zero
        player.seek(to: CMTime(seconds: t, preferredTimescale: 600), toleranceBefore: tolerance, toleranceAfter: tolerance)
    }

    func beginScrub() {
        isScrubbing = true
        if isPlaying { player.pause(); isPlaying = false }
    }

    func endScrub() {
        isScrubbing = false
        seek(to: currentTime)
    }

    // MARK: Selection helpers

    var selectedClipIndex: Int? {
        guard case .clip(let id) = selection else { return nil }
        return project.clips.firstIndex { $0.id == id }
    }

    var selectedOverlayIndex: Int? {
        guard case .overlay(let id) = selection else { return nil }
        return project.overlays.firstIndex { $0.id == id }
    }

    var selectedTextIndex: Int? {
        guard case .text(let id) = selection else { return nil }
        return project.texts.firstIndex { $0.id == id }
    }

    var selectedAudioIndex: Int? {
        guard case .audio(let id) = selection else { return nil }
        return project.audios.firstIndex { $0.id == id }
    }

    /// The selected clip or overlay's media, as a value.
    var selectedMedia: Clip? {
        if let i = selectedClipIndex { return project.clips[i] }
        if let i = selectedOverlayIndex { return project.overlays[i].clip }
        return nil
    }

    var selectedIsOverlay: Bool { selectedOverlayIndex != nil }

    /// Seconds into the selected media at the playhead.
    var selectedLocalTime: Double? {
        if let i = selectedClipIndex { return currentTime - project.start(ofClipAt: i) }
        if let i = selectedOverlayIndex { return currentTime - project.overlays[i].start }
        return nil
    }

    /// Edits the selected clip or overlay's media.
    func editMedia(_ change: (inout Clip) -> Void) {
        if let i = selectedClipIndex {
            let id = project.clips[i].id
            edit { p in if let j = p.clips.firstIndex(where: { $0.id == id }) { change(&p.clips[j]) } }
        } else if let i = selectedOverlayIndex {
            let id = project.overlays[i].id
            edit { p in if let j = p.overlays.firstIndex(where: { $0.id == id }) { change(&p.overlays[j].clip) } }
        }
    }

    func mediaBinding<T>(_ key: WritableKeyPath<Clip, T>, fallback: T) -> Binding<T> {
        Binding(get: { self.selectedMedia?[keyPath: key] ?? fallback },
                set: { v in self.editMedia { $0[keyPath: key] = v } })
    }

    func overlayBinding<T>(_ key: WritableKeyPath<OverlayItem, T>, fallback: T) -> Binding<T> {
        Binding(get: { self.selectedOverlayIndex.map { self.project.overlays[$0][keyPath: key] } ?? fallback },
                set: { v in
                    guard let i = self.selectedOverlayIndex else { return }
                    let id = self.project.overlays[i].id
                    self.edit { p in if let j = p.overlays.firstIndex(where: { $0.id == id }) { p.overlays[j][keyPath: key] = v } }
                })
    }

    func textBinding<T>(_ key: WritableKeyPath<TextOverlay, T>, fallback: T) -> Binding<T> {
        Binding(get: { self.selectedTextIndex.map { self.project.texts[$0][keyPath: key] } ?? fallback },
                set: { v in
                    guard let i = self.selectedTextIndex else { return }
                    let id = self.project.texts[i].id
                    self.edit { p in if let j = p.texts.firstIndex(where: { $0.id == id }) { p.texts[j][keyPath: key] = v } }
                })
    }

    func audioBinding<T>(_ key: WritableKeyPath<AudioItem, T>, fallback: T) -> Binding<T> {
        Binding(get: { self.selectedAudioIndex.map { self.project.audios[$0][keyPath: key] } ?? fallback },
                set: { v in
                    guard let i = self.selectedAudioIndex else { return }
                    let id = self.project.audios[i].id
                    self.edit { p in if let j = p.audios.firstIndex(where: { $0.id == id }) { p.audios[j][keyPath: key] = v } }
                })
    }

    /// Selects the main clip under the playhead if no clip or overlay is selected.
    @discardableResult
    func selectClipAtPlayhead() -> Bool {
        if selectedMedia != nil { return true }
        guard let (i, _) = project.clip(at: currentTime) else { return false }
        selection = .clip(project.clips[i].id)
        return true
    }

    // MARK: Transform and keyframes

    /// A transform property of the selected media at the playhead. With keyframes,
    /// changing it records a keyframe here, the way animation works in most editors.
    func transformBinding(_ key: WritableKeyPath<Transform, Double>) -> Binding<Double> {
        Binding(get: {
            guard let clip = self.selectedMedia, let local = self.selectedLocalTime else { return Transform()[keyPath: key] }
            return clip.transform(atLocal: local)[keyPath: key]
        }, set: { v in
            guard let local = self.selectedLocalTime else { return }
            self.editMedia { clip in
                if clip.keyframes.isEmpty {
                    clip.transform[keyPath: key] = v
                } else {
                    var value = clip.transform(atLocal: local)
                    value[keyPath: key] = v
                    Self.upsertKeyframe(&clip, at: local, value: value)
                }
            }
        })
    }

    private static func upsertKeyframe(_ clip: inout Clip, at local: Double, value: Transform) {
        let t = max(0, min(clip.duration, local))
        if let k = clip.keyframes.firstIndex(where: { abs($0.time - t) < 0.06 }) {
            clip.keyframes[k].value = value
        } else {
            clip.keyframes.append(Keyframe(time: t, value: value))
            clip.keyframes.sort { $0.time < $1.time }
        }
    }

    var keyframeAtPlayhead: Bool {
        guard let clip = selectedMedia, let local = selectedLocalTime else { return false }
        return clip.keyframes.contains { abs($0.time - local) < 0.06 }
    }

    /// The curve leaving the keyframe at the playhead, if there is one.
    var keyframeEasingBinding: Binding<KeyframeEasing>? {
        guard let clip = selectedMedia, let local = selectedLocalTime,
              let k = clip.keyframes.first(where: { abs($0.time - local) < 0.06 }) else { return nil }
        let id = k.id
        return Binding(get: { self.selectedMedia?.keyframes.first { $0.id == id }?.easing ?? .smooth },
                       set: { v in self.editMedia { c in if let i = c.keyframes.firstIndex(where: { $0.id == id }) { c.keyframes[i].easing = v } } })
    }

    func toggleKeyframe() {
        guard let local = selectedLocalTime, let clip = selectedMedia, local >= -0.01, local <= clip.duration + 0.01 else { return }
        editMedia { clip in
            if let k = clip.keyframes.firstIndex(where: { abs($0.time - local) < 0.06 }) {
                clip.keyframes.remove(at: k)
            } else {
                Self.upsertKeyframe(&clip, at: local, value: clip.transform(atLocal: local))
            }
        }
    }

    func clearKeyframes() {
        editMedia { clip in
            if let first = clip.keyframes.first { clip.transform = first.value }
            clip.keyframes.removeAll()
        }
    }

    // MARK: Operations

    func addClips(_ clips: [Clip]) {
        guard !clips.isEmpty else { return }
        edit { p in
            if let at = p.clip(at: currentTime), !p.clips.isEmpty {
                p.clips.insert(contentsOf: clips, at: at.index + 1)
            } else {
                p.clips.append(contentsOf: clips)
            }
        }
    }

    func addOverlays(_ clips: [Clip]) {
        guard !clips.isEmpty else { return }
        var added: [OverlayItem] = []
        for (n, c) in clips.enumerated() {
            var clip = c
            clip.transform.scale = 0.55
            clip.transform.x = Double(n) * 0.04
            clip.transform.y = Double(n) * 0.04
            if clip.isPhoto { clip.trimEnd = min(clip.sourceDuration, max(1, duration - currentTime)) }
            added.append(OverlayItem(clip: clip, start: currentTime))
        }
        edit { $0.overlays.append(contentsOf: added) }
        selection = added.last.map { .overlay($0.id) }
    }

    func splitAtPlayhead() {
        switch selection {
        case .overlay(let id):
            guard let i = project.overlays.firstIndex(where: { $0.id == id }) else { return }
            let item = project.overlays[i]
            let local = currentTime - item.start
            guard local > 0.1, local < item.duration - 0.1 else { return }
            let cut = item.clip.sourceTime(atLocal: local)
            var first = item, second = item
            first.clip.trimEnd = cut
            second.id = UUID(); second.clip.id = UUID(); second.clip.trimStart = cut; second.start = currentTime
            second.clip.keyframes = []
            edit { p in p.overlays.replaceSubrange(i...i, with: [first, second]) }
            selection = .overlay(second.id)
        case .text(let id):
            guard let i = project.texts.firstIndex(where: { $0.id == id }) else { return }
            let t = project.texts[i]
            guard currentTime > t.start + 0.1, currentTime < t.end - 0.1 else { return }
            var first = t, second = t
            first.duration = currentTime - t.start
            second.id = UUID(); second.start = currentTime; second.duration = t.end - currentTime
            edit { p in p.texts.replaceSubrange(i...i, with: [first, second]) }
            selection = .text(second.id)
        case .audio(let id):
            guard let i = project.audios.firstIndex(where: { $0.id == id }) else { return }
            let a = project.audios[i]
            let local = currentTime - a.start
            guard local > 0.1, local < a.length - 0.1, !a.loop else { return }
            var first = a, second = a
            first.trimEnd = a.trimStart + local; first.fadeOut = 0
            second.id = UUID(); second.trimStart = a.trimStart + local; second.start = currentTime; second.fadeIn = 0
            edit { p in p.audios.replaceSubrange(i...i, with: [first, second]) }
            selection = .audio(second.id)
        default:
            guard let (i, start) = project.clip(at: currentTime) else { return }
            let clip = project.clips[i]
            let local = currentTime - start
            guard local > 0.1, local < clip.duration - 0.1 else { return }
            let cut = clip.sourceTime(atLocal: local)
            var first = clip, second = clip
            first.trimEnd = cut; first.fadeOut = 0
            first.keyframes = clip.keyframes.filter { $0.time <= local }
            second.id = UUID(); second.trimStart = cut; second.fadeIn = 0; second.transitionIn = Transition()
            second.keyframes = clip.keyframes.filter { $0.time > local }.map { var k = $0; k.time -= local; return k }
            if !clip.keyframes.isEmpty { second.transform = clip.transform(atLocal: local) }
            edit { p in p.clips.replaceSubrange(i...i, with: [first, second]) }
            selection = .clip(second.id)
        }
    }

    func deleteSelection() {
        switch selection {
        case .clip(let id):
            // Ripple: what sits after the clip moves back with the cut, unless switched off.
            let ripple = UserDefaults.standard.object(forKey: "rippleEdits") as? Bool ?? true
            edit { p in
                guard let i = p.clips.firstIndex(where: { $0.id == id }) else { return }
                let end = p.start(ofClipAt: i) + p.clips[i].duration
                let before = p.duration
                p.clips.remove(at: i)
                let delta = before - p.duration
                guard ripple, delta > 0.001 else { return }
                for j in p.overlays.indices where p.overlays[j].start >= end - 0.01 { p.overlays[j].start -= delta }
                for j in p.texts.indices where p.texts[j].start >= end - 0.01 { p.texts[j].start -= delta }
                for j in p.audios.indices where p.audios[j].start >= end - 0.01 && !p.audios[j].loop { p.audios[j].start -= delta }
            }
        case .overlay(let id): edit { $0.overlays.removeAll { $0.id == id } }
        case .text(let id): edit { $0.texts.removeAll { $0.id == id } }
        case .audio(let id): edit { $0.audios.removeAll { $0.id == id } }
        case nil: return
        }
        selection = nil
        if currentTime > duration { seek(to: duration) }
    }

    func duplicateSelection() {
        switch selection {
        case .clip(let id):
            guard let i = project.clips.firstIndex(where: { $0.id == id }) else { return }
            var copy = project.clips[i]; copy.id = UUID()
            edit { $0.clips.insert(copy, at: i + 1) }
            selection = .clip(copy.id)
        case .overlay(let id):
            guard let o = project.overlays.first(where: { $0.id == id }) else { return }
            var copy = o; copy.id = UUID(); copy.clip.id = UUID(); copy.start = min(o.end, max(0, duration - 0.5))
            edit { $0.overlays.append(copy) }
            selection = .overlay(copy.id)
        case .text(let id):
            guard let t = project.texts.first(where: { $0.id == id }) else { return }
            var copy = t; copy.id = UUID(); copy.start = t.end
            edit { $0.texts.append(copy) }
            selection = .text(copy.id)
        case .audio(let id):
            guard let a = project.audios.first(where: { $0.id == id }) else { return }
            var copy = a; copy.id = UUID(); copy.start = a.start + a.length; copy.loop = false
            edit { $0.audios.append(copy) }
            selection = .audio(copy.id)
        case nil: break
        }
    }

    func moveSelectedClip(by offset: Int) {
        guard let i = selectedClipIndex else { return }
        let j = i + offset
        guard project.clips.indices.contains(j) else { return }
        edit { $0.clips.swapAt(i, j) }
    }

    func setTransition(_ transition: Transition, forClipAt index: Int) {
        guard project.clips.indices.contains(index), index > 0 else { return }
        edit { $0.clips[index].transitionIn = transition }
    }

    func applyTransitionToAll(_ transition: Transition) {
        edit { p in for i in p.clips.indices.dropFirst() { p.clips[i].transitionIn = transition } }
    }

    /// Freeze the frame under the playhead as a 2 s still after it.
    func freezeFrame() async {
        guard let (i, start) = project.clip(at: currentTime) else { return }
        let clip = project.clips[i]
        let source = clip.sourceTime(atLocal: currentTime - start)
        let url = ProjectStore.mediaFolder(for: project.id).appendingPathComponent(clip.file)
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
        generator.appliesPreferredTrackTransform = true
        generator.requestedTimeToleranceBefore = .zero
        generator.requestedTimeToleranceAfter = .zero
        do {
            let (cg, _) = try await generator.image(at: CMTime(seconds: source, preferredTimescale: 600))
            var still = try await MediaImporter.importPhoto(UIImage(cgImage: cg), into: project.id)
            still.trimEnd = 2
            still.filter = clip.filter; still.fill = clip.fill; still.effects = clip.effects; still.crop = clip.crop
            let offset = currentTime - start
            let splits = offset > 0.1 && offset < clip.duration - 0.1
            if splits { selection = nil; splitAtPlayhead() }
            let insertAt = splits ? i + 1 : (offset < clip.duration / 2 ? i : i + 1)
            edit { p in p.clips.insert(still, at: min(p.clips.count, insertAt)) }
            selection = .clip(still.id)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    /// Moves a clip's sound to its own audio track and mutes the clip.
    func extractAudio() {
        guard let i = selectedClipIndex ?? project.clip(at: currentTime)?.index else { return }
        let clip = project.clips[i]
        guard clip.hasAudio, !clip.isPhoto else { errorMessage = "This clip has no sound."; return }
        let start = project.start(ofClipAt: i)
        let item = AudioItem(file: clip.file, name: "Audio from clip \(i + 1)", kind: .extracted, start: start,
                             sourceDuration: clip.sourceDuration, trimStart: clip.trimStart,
                             trimEnd: clip.speedCurve == .none && clip.speed == 1 ? clip.trimEnd : clip.trimStart + clip.duration,
                             volume: clip.volume, effect: clip.audioEffect, cleanup: clip.cleanup)
        edit { p in
            p.clips[i].volume = 0
            p.audios.append(item)
        }
        selection = .audio(item.id)
    }

    // MARK: Text, shapes and audio

    func addText(_ preset: TextOverlay? = nil) {
        var t = preset ?? TextOverlay(text: "Your text")
        t.id = UUID()
        t.start = currentTime
        t.duration = min(3, max(1, duration - currentTime))
        edit { $0.texts.append(t) }
        selection = .text(t.id)
    }

    func addShape(_ kind: ShapeKind) {
        var t = TextOverlay(text: "", color: RGBA(r: 0, g: 0.82, b: 0.78), accent: .white, size: 0.05, style: .plain)
        t.shape = kind
        t.start = currentTime
        t.duration = min(3, max(1, duration - currentTime))
        edit { $0.texts.append(t) }
        selection = .text(t.id)
    }

    func replaceCaptions(_ captions: [TextOverlay]) {
        edit { p in
            p.texts.removeAll(where: \.isCaption)
            p.texts.append(contentsOf: captions)
        }
    }

    /// Restyle every caption like the selected one.
    func applyStyleToAllCaptions(from source: TextOverlay) {
        edit { p in
            for i in p.texts.indices where p.texts[i].isCaption {
                p.texts[i].color = source.color; p.texts[i].accent = source.accent
                p.texts[i].size = source.size; p.texts[i].bold = source.bold; p.texts[i].font = source.font
                p.texts[i].customFont = source.customFont; p.texts[i].tracking = source.tracking
                p.texts[i].style = source.style; p.texts[i].x = source.x; p.texts[i].y = source.y
                p.texts[i].textCase = source.textCase; p.texts[i].animationIn = source.animationIn
            }
        }
    }

    func addAudio(_ item: AudioItem, atPlayhead: Bool) {
        var a = item
        if atPlayhead { a.start = currentTime }
        edit { $0.audios.append(a) }
        selection = .audio(a.id)
    }

    func setAspect(_ aspect: AspectRatio) { edit { $0.aspect = aspect } }
    func rename(_ name: String) { edit { $0.name = name } }
}
