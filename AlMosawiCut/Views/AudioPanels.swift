import AVFoundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// Voice effects and cleanup, for a clip's own sound or an audio item.
struct VoiceFXPanel: View {
    @Bindable var model: EditorModel
    @State private var category = "Voice"

    var body: some View {
        if model.selectedMedia != nil || model.selectedAudioIndex != nil {
            let effect = effectBinding
            let cleanup = cleanupBinding
            VStack(alignment: .leading, spacing: 12) {
                SectionLabel("Clean up")
                HStack(spacing: 6) {
                    toggle("Enhance voice", "person.wave.2.fill", Binding(get: { cleanup.wrappedValue.enhance }, set: { cleanup.wrappedValue.enhance = $0 }))
                    toggle("Reduce noise", "waveform.badge.minus", Binding(get: { cleanup.wrappedValue.denoise }, set: { cleanup.wrappedValue.denoise = $0 }))
                    toggle("Normalize", "chart.bar.fill", Binding(get: { cleanup.wrappedValue.normalize }, set: { cleanup.wrappedValue.normalize = $0 }))
                }
                SectionLabel("Effects")
                Chips(options: [("Voice", "Voice"), ("Space", "Space"), ("Tone", "Tone")], selection: $category)
                LazyVGrid(columns: gridColumns, spacing: 8) {
                    GridPick(icon: "nosign", label: "None", selected: effect.wrappedValue == .none) { effect.wrappedValue = .none }
                    ForEach(AudioEffect.allCases.filter { $0.category == category }) { e in
                        GridPick(icon: e.symbol, label: e.label, selected: effect.wrappedValue == e) { effect.wrappedValue = e }
                    }
                }
                Text("Effects render once per clip, so the first play after a change can take a moment.")
                    .font(.caption).foregroundStyle(Theme.secondaryText)
            }
        } else { NeedsSelection(text: "Select a clip or audio first.") }
    }

    private var effectBinding: Binding<AudioEffect> {
        model.selectedAudioIndex != nil ? model.audioBinding(\.effect, fallback: .none) : model.mediaBinding(\.audioEffect, fallback: .none)
    }

    private var cleanupBinding: Binding<VoiceCleanup> {
        model.selectedAudioIndex != nil ? model.audioBinding(\.cleanup, fallback: VoiceCleanup()) : model.mediaBinding(\.cleanup, fallback: VoiceCleanup())
    }

    private func toggle(_ title: String, _ icon: String, _ on: Binding<Bool>) -> some View {
        Button { on.wrappedValue.toggle() } label: {
            VStack(spacing: 5) {
                Image(systemName: icon).font(.system(size: 17))
                Text(title).font(.system(size: 10, weight: .semibold)).lineLimit(1).minimumScaleFactor(0.8)
            }
            .frame(maxWidth: .infinity, minHeight: 58)
            .background(on.wrappedValue ? Theme.accent.opacity(0.2) : Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(on.wrappedValue ? Theme.accent : .clear, lineWidth: 2))
            .foregroundStyle(on.wrappedValue ? Theme.accent : .white)
        }
        .buttonStyle(.plain)
    }
}

/// Brings your own sound into the project: music or sounds from Files,
/// the sound of a video in Photos, or a recording.
struct MusicPanel: View {
    @Bindable var model: EditorModel
    var open: (Panel) -> Void
    @State private var importingFile = false
    @State private var asMusic = true
    @State private var pickingVideo = false
    @State private var videoItem: PhotosPickerItem?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                source("Music", "Loops under the video", "music.note.list") { asMusic = true; importingFile = true }
                source("Sound", "Plays once, full volume", "speaker.wave.2.fill") { asMusic = false; importingFile = true }
                source("From a video", "Its sound, from Photos", "film") { pickingVideo = true }
                source("Record", "Voiceover with the mic", "mic.fill") { open(.record) }
            }
            Text("Audio goes on its own track at the playhead. Files can be songs (MP3, M4A, WAV, AIFF) or videos, whose sound is used.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
            if !model.project.audios.isEmpty {
                SectionLabel("In this project")
                ForEach(model.project.audios) { a in
                    Button {
                        model.selection = .audio(a.id)
                        open(.audioItem)
                    } label: {
                        HStack {
                            Image(systemName: a.kind.symbol)
                            Text(a.name).lineLimit(1)
                            Spacer()
                            Text(a.start.timecode).font(.caption.monospacedDigit()).foregroundStyle(Theme.secondaryText)
                            Image(systemName: "chevron.right").font(.caption)
                        }
                        .font(.subheadline)
                        .padding(12)
                        .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 10))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .fileImporter(isPresented: $importingFile, allowedContentTypes: [.audio, .movie], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            let music = asMusic
            Task {
                model.busyMessage = "Adding audio…"
                defer { model.busyMessage = nil }
                var failures: [String] = []
                for url in urls {
                    do {
                        let item = try await MediaImporter.importAudio(at: url, into: model.project.id, asMusic: music)
                        model.addAudio(item, atPlayhead: true)
                    } catch { failures.append("\(url.lastPathComponent): \(error.localizedDescription)") }
                }
                if !failures.isEmpty { model.errorMessage = failures.joined(separator: "\n") }
            }
        }
        .photosPicker(isPresented: $pickingVideo, selection: $videoItem, matching: .videos, preferredItemEncoding: .current)
        .onChange(of: videoItem) { _, item in
            guard let item else { return }
            Task {
                model.busyMessage = "Adding audio…"
                defer { model.busyMessage = nil; videoItem = nil }
                do {
                    guard let movie = try await item.loadTransferable(type: PickedMovie.self) else { throw CocoaError(.fileReadUnknown) }
                    let audio = try await MediaImporter.importAudio(fromMovie: movie.url, into: model.project.id)
                    model.addAudio(audio, atPlayhead: true)
                } catch { model.errorMessage = error.localizedDescription }
            }
        }
    }

    private func source(_ title: String, _ detail: String, _ icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: icon).font(.system(size: 18, weight: .semibold))
                    .frame(width: 36, height: 36)
                    .background(Theme.accent.opacity(0.18), in: RoundedRectangle(cornerRadius: 10))
                    .foregroundStyle(Theme.accent)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.subheadline.weight(.bold))
                    Text(detail).font(.system(size: 10)).foregroundStyle(Theme.secondaryText).lineLimit(1).minimumScaleFactor(0.8)
                }
                Spacer(minLength: 0)
            }
            .padding(10)
            .frame(maxWidth: .infinity)
            .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
        }
        .buttonStyle(PressStyle())
    }
}

/// Records a voiceover at the playhead, optionally while the video plays (muted).
struct RecordPanel: View {
    @Bindable var model: EditorModel
    @State private var recorder = VoiceRecorder()
    @State private var startedAt: Double = 0
    @AppStorage("recordPlayAlong") private var playAlong = true

    var body: some View {
        VStack(spacing: 14) {
            Text(recorder.isRecording ? recorder.elapsed.timecode : "Starts at \(model.currentTime.timecode)")
                .font(.system(size: 28, weight: .bold).monospacedDigit())
                .foregroundStyle(recorder.isRecording ? Color(red: 1, green: 0.35, blue: 0.35) : .white)
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Theme.surfaceHigh)
                    Capsule().fill(Theme.accent).frame(width: geo.size.width * recorder.level)
                }
            }
            .frame(height: 6)
            .animation(.linear(duration: 0.05), value: recorder.level)
            Button {
                Task {
                    if recorder.isRecording { await finish() } else { await begin() }
                }
            } label: {
                ZStack {
                    Circle().stroke(Color.white, lineWidth: 4).frame(width: 70, height: 70)
                    if recorder.isRecording {
                        RoundedRectangle(cornerRadius: 6).fill(Color.red).frame(width: 28, height: 28)
                    } else {
                        Circle().fill(Color.red).frame(width: 56, height: 56)
                    }
                }
            }
            .buttonStyle(PressStyle())
            .disabled(model.project.clips.isEmpty)
            .accessibilityLabel(recorder.isRecording ? "Stop recording" : "Start recording")
            Toggle("Play the video while recording (muted)", isOn: $playAlong)
                .font(.subheadline)
                .disabled(recorder.isRecording)
            Text(model.project.clips.isEmpty ? "Add a clip first." : "Use headphones to keep the video's sound out of the recording.")
                .font(.caption).foregroundStyle(Theme.secondaryText)
        }
        .onDisappear { if recorder.isRecording { Task { await finish() } } }
    }

    private func begin() async {
        do {
            startedAt = model.currentTime
            try await recorder.start(in: ProjectStore.mediaFolder(for: model.project.id))
            if playAlong, model.currentTime < model.duration - 0.1 {
                model.player.isMuted = true
                if !model.isPlaying { model.togglePlay() }
            }
        } catch {
            model.errorMessage = error.localizedDescription
        }
    }

    private func finish() async {
        if model.isPlaying { model.togglePlay() }
        model.player.isMuted = false
        guard let file = recorder.stop() else { return }
        let url = ProjectStore.mediaFolder(for: model.project.id).appendingPathComponent(file)
        let length = (try? await AVURLAsset(url: url).load(.duration).seconds) ?? 0
        guard length > 0.2 else { try? FileManager.default.removeItem(at: url); return }
        let n = model.project.audios.filter { $0.kind == .voiceover }.count + 1
        let item = AudioItem(file: file, name: "Voiceover \(n)", kind: .voiceover, start: startedAt,
                             sourceDuration: length, trimEnd: length)
        model.addAudio(item, atPlayhead: false)
    }
}

struct AudioItemPanel: View {
    @Bindable var model: EditorModel
    var body: some View {
        if let i = model.selectedAudioIndex {
            let item = model.project.audios[i]
            let start = model.audioBinding(\.trimStart, fallback: 0)
            let end = model.audioBinding(\.trimEnd, fallback: 1)
            VStack(alignment: .leading, spacing: 12) {
                Label(item.name, systemImage: item.kind.symbol).font(.subheadline.weight(.bold))
                LabeledSlider(title: "Volume", value: model.audioBinding(\.volume, fallback: 1), range: 0...2, format: { "\(Int($0 * 100))%" }, reset: 1)
                LabeledSlider(title: "Fade in", value: model.audioBinding(\.fadeIn, fallback: 0), range: 0...5, format: { String(format: "%.1fs", $0) }, reset: 0)
                LabeledSlider(title: "Fade out", value: model.audioBinding(\.fadeOut, fallback: 0), range: 0...5, format: { String(format: "%.1fs", $0) }, reset: 0)
                Toggle("Loop to fill the video", isOn: model.audioBinding(\.loop, fallback: false)).font(.subheadline)
                LabeledSlider(title: "Starts on timeline at", value: model.audioBinding(\.start, fallback: 0), range: 0...max(0.1, model.duration), format: { $0.timecode })
                LabeledSlider(title: "Trim start", value: Binding(get: { start.wrappedValue }, set: { start.wrappedValue = min($0, end.wrappedValue - 0.2) }),
                              range: 0...max(0.3, item.sourceDuration), format: { $0.timecode })
                LabeledSlider(title: "Trim end", value: Binding(get: { end.wrappedValue }, set: { end.wrappedValue = max($0, start.wrappedValue + 0.2) }),
                              range: 0...max(0.3, item.sourceDuration), format: { $0.timecode })
            }
        } else { NeedsSelection(text: "Select an audio item on the timeline.") }
    }
}

/// Text to speech with the device's voices, including your own Personal Voice.
struct SpeechPanel: View {
    @Bindable var model: EditorModel
    @AppStorage("ttsLanguage") private var language = "ar"
    @AppStorage("ttsVoice") private var voiceID = ""
    @State private var text = ""
    @State private var rate: Double = 0.5
    @State private var pitch: Double = 1
    @State private var addCaptions = true
    @State private var maker = SpeechMaker()
    @State private var voices: [AVSpeechSynthesisVoice] = []
    @State private var personalStatus = ""

    private let languages = [("ar", "العربية"), ("en", "English"), ("fr", "Français"), ("es", "Español"), ("de", "Deutsch"),
                             ("tr", "Türkçe"), ("ur", "اردو"), ("hi", "हिन्दी"), ("fa", "فارسی")]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField("Type what the voice should say", text: $text, axis: .vertical)
                .lineLimit(2...5)
                .padding(12)
                .background(Theme.surfaceHigh, in: RoundedRectangle(cornerRadius: 12))
            Chips(options: languages, selection: $language)
            HStack {
                Text("Voice").font(.caption.weight(.medium)).foregroundStyle(Theme.secondaryText)
                Spacer()
                Picker("Voice", selection: $voiceID) {
                    Text("Default").tag("")
                    ForEach(voices, id: \.identifier) { v in
                        Text(label(v)).tag(v.identifier)
                    }
                }
                .tint(.white)
            }
            LabeledSlider(title: "Speed", value: $rate, range: 0.3...0.65, format: { String(format: "%.0f%%", $0 / 0.5 * 100) }, reset: 0.5)
            LabeledSlider(title: "Pitch", value: $pitch, range: 0.6...1.6, format: { String(format: "%.2f", $0) }, reset: 1)
            Toggle("Also add captions", isOn: $addCaptions).font(.subheadline)
            Button { Task { await generate() } } label: {
                Label("Add voiceover at playhead", systemImage: "text.bubble.fill")
                    .font(.subheadline.weight(.bold))
                    .frame(maxWidth: .infinity, minHeight: 42)
                    .background(Theme.accent, in: Capsule())
                    .foregroundStyle(.black)
            }
            .buttonStyle(.plain)
            .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            Button {
                AVSpeechSynthesizer.requestPersonalVoiceAuthorization { status in
                    Task { @MainActor in
                        personalStatus = status == .authorized
                            ? "Personal Voice allowed. Your voice is now in the list."
                            : "Personal Voice isn't available. Create one in Settings › Accessibility › Personal Voice."
                        loadVoices()
                    }
                }
            } label: {
                Label("Use my Personal Voice", systemImage: "person.wave.2.fill").font(.caption.weight(.bold))
            }
            .buttonStyle(.plain).foregroundStyle(Theme.accent)
            if !personalStatus.isEmpty { Text(personalStatus).font(.caption).foregroundStyle(Theme.secondaryText) }
        }
        .onAppear(perform: loadVoices)
        .onChange(of: language) { _, _ in voiceID = ""; loadVoices() }
    }

    private func label(_ v: AVSpeechSynthesisVoice) -> String {
        if v.voiceTraits.contains(.isPersonalVoice) { return "\(v.name) (Personal)" }
        let quality = v.quality == .premium ? " ★★" : v.quality == .enhanced ? " ★" : ""
        return v.name + quality
    }

    private func loadVoices() { voices = SpeechMaker.voices(language: language) }

    private func generate() async {
        let spoken = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !spoken.isEmpty else { return }
        model.busyMessage = "Making voiceover…"
        defer { model.busyMessage = nil }
        let name = UUID().uuidString + ".caf"
        let url = ProjectStore.mediaFolder(for: model.project.id).appendingPathComponent(name)
        do {
            let voice = voices.first { $0.identifier == voiceID }
            try await maker.write(spoken, voice: voice, language: language, rate: Float(rate), pitch: Float(pitch), to: url)
            let length = try await AVURLAsset(url: url).load(.duration).seconds
            let start = model.currentTime
            let item = AudioItem(file: name, name: String(spoken.prefix(24)), kind: .voiceover, start: start,
                                 sourceDuration: length, trimEnd: length, volume: 1)
            model.addAudio(item, atPlayhead: true)
            if addCaptions { model.edit { $0.texts.append(contentsOf: Self.captions(for: spoken, start: start, length: length)) } }
            text = ""
        } catch {
            model.errorMessage = "Couldn't make the voiceover: \(error.localizedDescription)"
        }
    }

    /// Splits the script into short captions timed by their share of the characters.
    static func captions(for text: String, start: Double, length: Double) -> [TextOverlay] {
        let words = text.split(whereSeparator: \.isWhitespace).map(String.init)
        var chunks: [String] = []
        var current: [String] = []
        for w in words {
            current.append(w)
            if current.count >= 4 || w.last.map({ ".!?،؟".contains($0) }) == true { chunks.append(current.joined(separator: " ")); current = [] }
        }
        if !current.isEmpty { chunks.append(current.joined(separator: " ")) }
        let totalChars = max(1, chunks.reduce(0) { $0 + $1.count })
        var t = start
        return chunks.map { chunk in
            let d = length * Double(chunk.count) / Double(totalChars)
            defer { t += d }
            return TextOverlay(text: chunk, start: t, duration: d, color: .white, accent: .black, size: 0.07, style: .outline,
                               animationIn: .pop, y: 0.74, isCaption: true)
        }
    }
}
