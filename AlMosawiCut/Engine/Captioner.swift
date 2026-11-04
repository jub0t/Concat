import AVFoundation
import Speech

/// Auto-captions: transcribe the edit's own sound on device and turn the
/// words into short timed titles.
enum Captioner {
    struct Word: Sendable { let text: String; let start: Double; let duration: Double }

    enum CaptionError: LocalizedError {
        case notAllowed, unavailable(String), noSpeech, noSound, timedOut
        var errorDescription: String? {
            switch self {
            case .timedOut: "Apple's speech service didn't answer. Check the internet connection, or install the language for on-device use under Settings › General › Keyboard › Dictation Languages, then try again."
            case .noSound: "This edit has no sound to caption. Unmute a clip, or add a voiceover."
            case .notAllowed: "Speech recognition is off for AlMosawiCut. Turn it on in Settings."
            case .unavailable(let lang): "Speech recognition for \(lang) isn't available on this device. Try adding it under Settings › General › Keyboard › Dictation Languages."
            case .noSpeech: "No speech was found in the video."
            }
        }
    }

    /// How the words are heard.
    enum Engine: String, CaseIterable, Identifiable {
        /// Whisper on the iPhone: no permissions, offline once the model is downloaded.
        case whisper
        /// Apple's speech recognizer: asks for Speech Recognition permission and needs Dictation.
        case apple
        var id: String { rawValue }
        var label: String { self == .whisper ? "Whisper on iPhone" : "Apple speech" }
    }

    static let languages: [(id: String, name: String)] = [
        ("auto", "Auto"), ("ar-SA", "العربية"), ("en-US", "English"), ("fr-FR", "Français"), ("es-ES", "Español"),
        ("de-DE", "Deutsch"), ("tr-TR", "Türkçe"), ("ur-PK", "اردو"), ("hi-IN", "हिन्दी"),
    ]

    /// Server recognition stops at about a minute, so longer edits are heard in pieces.
    private static let chunkLength = 50.0

    static func captions(for project: Project, locale: String, wordsPerCaption: Int,
                         engine: Engine = .whisper, model: WhisperEngine.Size = .small,
                         progress: @escaping @MainActor (String) -> Void = { _ in }) async throws -> [TextOverlay] {
        if engine == .whisper {
            // Straight from the edit's sound file: no microphone, permission or speech service.
            await progress("Getting the sound from your edit…")
            let audio = try await exportVoice(of: project)
            defer { try? FileManager.default.removeItem(at: audio) }
            let length = try await AVURLAsset(url: audio).load(.duration).seconds
            let code = locale == "auto" ? nil : String(locale.prefix { $0 != "-" })
            let words = try await WhisperEngine.shared.transcribe(audio, language: code, size: model, progress: progress)
            guard !words.isEmpty else { throw CaptionError.noSpeech }
            return group(timed(words, over: length), max: wordsPerCaption)
        }
        let locale = locale == "auto" ? (Locale.current.language.languageCode.map { $0.identifier } ?? "en-US") : locale
        await progress("Checking speech recognition…")
        let status = await withCheckedContinuation { c in SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0) } }
        guard status == .authorized else { throw CaptionError.notAllowed }
        guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: locale)), recognizer.isAvailable else {
            throw CaptionError.unavailable(locale)
        }

        await progress("Getting the sound from your edit…")
        let audio = try await exportVoice(of: project)
        defer { try? FileManager.default.removeItem(at: audio) }
        let length = try await AVURLAsset(url: audio).load(.duration).seconds

        // Prefer on-device, which has no length limit; a device can claim support
        // before its model is downloaded, so fall back to Apple's service, in pieces.
        let words: [Word]
        do {
            if recognizer.supportsOnDeviceRecognition {
                await progress("Listening on this device…")
                // The on-device model can be missing or still downloading and never answer.
                do { words = try await recognize(audio, with: recognizer, onDevice: true, timeout: max(25, length * 1.5 + 10)) }
                catch { words = try await recognizeInPieces(audio, length: length, with: recognizer, progress: progress) }
            } else {
                words = try await recognizeInPieces(audio, length: length, with: recognizer, progress: progress)
            }
        } catch let error as NSError where error.domain == "kLSRErrorDomain" && error.code == 300
                    || error.domain == "kAFAssistantErrorDomain" && error.code == 1101 {
            // The speech service has no model for this language on this device.
            throw CaptionError.unavailable(Locale.current.localizedString(forIdentifier: locale) ?? locale)
        }
        guard !words.isEmpty else { throw CaptionError.noSpeech }
        return group(timed(words, over: length), max: wordsPerCaption)
    }

    /// Sends the sound to Apple's recognizer a piece at a time and puts the words back on one clock.
    private static func recognizeInPieces(_ audio: URL, length: Double, with recognizer: SFSpeechRecognizer,
                                          progress: @MainActor (String) -> Void) async throws -> [Word] {
        guard length > chunkLength + 5 else {
            await progress("Listening…")
            do { return try await recognize(audio, with: recognizer, onDevice: false, timeout: 90) }
            catch let error as NSError where error.domain == "kAFAssistantErrorDomain" && error.code == 1110 { return [] }
        }
        let asset = AVURLAsset(url: audio)
        let count = Int(ceil(length / chunkLength))
        var words: [Word] = []
        for i in 0..<count {
            await progress("Listening… part \(i + 1) of \(count)")
            let start = Double(i) * chunkLength
            let piece = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".m4a")
            defer { try? FileManager.default.removeItem(at: piece) }
            guard let session = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetAppleM4A) else {
                throw CocoaError(.featureUnsupported)
            }
            session.timeRange = CMTimeRange(start: CMTime(seconds: start, preferredTimescale: 600),
                                            duration: CMTime(seconds: min(chunkLength, length - start), preferredTimescale: 600))
            try await session.export(to: piece, as: .m4a)
            do {
                let pieceLength = min(chunkLength, length - start)
                let raw = try await recognize(piece, with: recognizer, onDevice: false, timeout: 90)
                let heard = timed(raw, over: pieceLength)
                words += heard.map { Word(text: $0.text, start: $0.start + start, duration: $0.duration) }
            } catch let error as NSError where error.domain == "kAFAssistantErrorDomain" && (error.code == 1110 || error.code == 203) {
                continue  // nothing spoken in this piece
            }
        }
        return words
    }

    /// Recognizes one file, giving up after `timeout` seconds instead of waiting forever.
    private static func recognize(_ audio: URL, with recognizer: SFSpeechRecognizer, onDevice: Bool,
                                  timeout: Double) async throws -> [Word] {
        try await withThrowingTaskGroup(of: [Word].self) { group in
            group.addTask { try await recognizeOnce(audio, with: recognizer, onDevice: onDevice) }
            group.addTask {
                try await Task.sleep(for: .seconds(timeout))
                throw CaptionError.timedOut
            }
            defer { group.cancelAll() }
            guard let words = try await group.next() else { throw CaptionError.timedOut }
            return words
        }
    }

    /// Holds a running recognition so a timeout can stop it and resume its waiter exactly once.
    private final class Recognition: @unchecked Sendable {
        private let lock = NSLock()
        private var continuation: CheckedContinuation<[Word], Error>?
        private var task: SFSpeechRecognitionTask?
        private var finished = false

        func start(_ c: CheckedContinuation<[Word], Error>, _ make: () -> SFSpeechRecognitionTask) {
            lock.lock()
            if finished { lock.unlock(); c.resume(throwing: CancellationError()); return }
            continuation = c
            lock.unlock()
            let t = make()
            lock.lock(); task = t; lock.unlock()
        }

        /// The latest words that came with real times (partial results keep them).
        private var timedWords: [Word] = []
        func remember(_ words: [Word]) { lock.lock(); timedWords = words; lock.unlock() }
        var lastTimed: [Word] { lock.lock(); defer { lock.unlock() }; return timedWords }

        func finish(_ result: Result<[Word], Error>) {
            lock.lock()
            let c = continuation
            continuation = nil
            finished = true
            lock.unlock()
            c?.resume(with: result)
        }

        func cancel() {
            lock.lock(); let t = task; lock.unlock()
            t?.cancel()
            finish(.failure(CancellationError()))
        }
    }

    /// A segment can hold several words; spread its time across them.
    private static func words(in result: SFSpeechRecognitionResult) -> [Word] {
        result.bestTranscription.segments.flatMap { segment -> [Word] in
            let parts = segment.substring.split(whereSeparator: \.isWhitespace).map(String.init)
            let each = segment.duration / Double(max(1, parts.count))
            return parts.enumerated().map { i, part in
                Word(text: part, start: segment.timestamp + Double(i) * each, duration: each)
            }
        }
    }

    private static func recognizeOnce(_ audio: URL, with recognizer: SFSpeechRecognizer, onDevice: Bool) async throws -> [Word] {
        let request = SFSpeechURLRecognitionRequest(url: audio)
        // Partial results carry word times even when the final one arrives without them.
        request.shouldReportPartialResults = true
        request.addsPunctuation = true
        request.requiresOnDeviceRecognition = onDevice
        let recognition = Recognition()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { c in
                recognition.start(c) {
                    recognizer.recognitionTask(with: request) { result, error in
                        if let result {
                            let words = Self.words(in: result)
                            if Self.hasTimings(words) { recognition.remember(words) }
                            if result.isFinal {
                                let kept = recognition.lastTimed
                                recognition.finish(.success(Self.hasTimings(words) || kept.isEmpty ? words : kept))
                                return
                            }
                        }
                        if let error {
                            // Keep what was heard before a late failure.
                            let kept = recognition.lastTimed
                            recognition.finish(kept.isEmpty ? .failure(error) : .success(kept))
                        }
                    }
                }
            }
        } onCancel: {
            recognition.cancel()
        }
    }

    /// True when the recognizer gave the words real times, not all zero.
    static func hasTimings(_ words: [Word]) -> Bool {
        guard let first = words.first, let last = words.last else { return false }
        if words.count == 1 { return first.duration > 0.01 }
        return last.start - first.start > 0.05
    }

    /// Words with usable times: as recognized, or, when iOS left them all at
    /// zero, spread over the sound by their length so captions follow the video.
    static func timed(_ words: [Word], over length: Double) -> [Word] {
        guard !hasTimings(words), length > 0 else { return words }
        let weights = words.map { Double(max(1, $0.text.count)) + 1 }
        let total = weights.reduce(0, +)
        var t = 0.0
        return zip(words, weights).map { word, w in
            let d = length * w / total
            defer { t += d }
            return Word(text: word.text, start: t, duration: d)
        }
    }

    /// Chunks of up to `max` words, broken early on a pause or a sentence end.
    static func group(_ words: [Word], max: Int) -> [TextOverlay] {
        var out: [TextOverlay] = []
        var chunk: [Word] = []
        func flush() {
            guard let first = chunk.first, let last = chunk.last else { return }
            let end = last.start + Swift.max(last.duration, 0.25)
            var caption = TextOverlay(
                text: chunk.map(\.text).joined(separator: " "),
                start: first.start, duration: Swift.max(0.3, end - first.start),
                color: .white, accent: .black, size: 0.075, bold: true, style: .outline,
                x: 0.5, y: 0.72, isCaption: true)
            caption.wordStarts = chunk.map { Swift.max(0, $0.start - first.start) }
            out.append(caption)
            chunk.removeAll()
        }
        for word in words {
            if let last = chunk.last, word.start - (last.start + last.duration) > 0.6 { flush() }
            chunk.append(word)
            if chunk.count >= max || word.text.last.map({ ".?!،؟".contains($0) }) == true { flush() }
        }
        flush()
        // Hold each caption until the next one starts if the gap is small, and
        // end it where the next begins, but never shorter than half a second.
        out.sort { $0.start < $1.start }
        for i in out.indices.dropLast() {
            let gap = out[i + 1].start - out[i].end
            if gap > 0, gap < 0.4 { out[i].duration += gap }
            if out[i].end > out[i + 1].start { out[i].duration = out[i + 1].start - out[i].start }
            if out[i].duration < 0.5 {
                out[i].duration = 0.5
                if out[i + 1].start < out[i].end {
                    let shift = out[i].end - out[i + 1].start
                    out[i + 1].start += shift
                    out[i + 1].duration = Swift.max(0.5, out[i + 1].duration - shift)
                }
            }
        }
        return out
    }

    /// The edit's spoken sound on the timeline's clock: clips, plus voiceovers and
    /// audio pulled out of clips. Music and sound effects are left out, and voice
    /// effects are dropped so the words are clear.
    private static func exportVoice(of project: Project) async throws -> URL {
        var voice = project
        voice.tracks = TrackFlags()  // mute is for playback; captions still hear the speech
        for i in voice.clips.indices { voice.clips[i].audioEffect = .none }
        for i in voice.overlays.indices { voice.overlays[i].clip.audioEffect = .none }
        voice.audios = voice.audios
            .filter { $0.kind == .voiceover || $0.kind == .extracted }
            .map { var a = $0; a.effect = .none; return a }
        let built = try await CompositionBuilder.build(voice, includeAudioItems: true)
        guard !built.composition.tracks(withMediaType: .audio).isEmpty else { throw CaptionError.noSound }
        guard let session = AVAssetExportSession(asset: built.composition, presetName: AVAssetExportPresetAppleM4A) else {
            throw CocoaError(.featureUnsupported)
        }
        session.audioMix = built.audioMix
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".m4a")
        try await session.export(to: url, as: .m4a)
        return url
    }
}

/// SubRip (.srt) subtitles in and out, so captions travel to and from other editors and players.
enum SRT {
    static func export(_ captions: [TextOverlay]) -> String {
        captions.sorted { $0.start < $1.start }.enumerated().map { i, c in
            "\(i + 1)\n\(stamp(c.start)) --> \(stamp(c.end))\n\(c.text)\n"
        }
        .joined(separator: "\n")
    }

    static func stamp(_ t: Double) -> String {
        let ms = Int((max(0, t) * 1000).rounded())
        return String(format: "%02d:%02d:%02d,%03d", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
    }

    /// Reads cues; tolerates a BOM, CRLF line ends, "." for ",", and missing numbers.
    static func parse(_ text: String) -> [(start: Double, end: Double, text: String)] {
        let clean = text.replacingOccurrences(of: "\u{FEFF}", with: "")
            .replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
        var cues: [(Double, Double, String)] = []
        for block in clean.components(separatedBy: "\n\n") {
            let lines = block.split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
            guard let timeIndex = lines.firstIndex(where: { $0.contains("-->") }) else { continue }
            let parts = lines[timeIndex].components(separatedBy: "-->")
            guard parts.count == 2, let a = seconds(parts[0]), let b = seconds(parts[1]), b > a else { continue }
            let body = lines[(timeIndex + 1)...].joined(separator: "\n")
                .replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
            if !body.isEmpty { cues.append((a, b, body)) }
        }
        return cues
    }

    private static func seconds(_ s: String) -> Double? {
        let f = s.trimmingCharacters(in: .whitespaces).split(separator: " ").first.map(String.init) ?? ""
        let hms = f.replacingOccurrences(of: ",", with: ".").split(separator: ":").compactMap { Double($0) }
        switch hms.count {
        case 3: return hms[0] * 3600 + hms[1] * 60 + hms[2]
        case 2: return hms[0] * 60 + hms[1]
        default: return nil
        }
    }
}
