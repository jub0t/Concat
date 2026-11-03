import AVFoundation

/// Renders a file's sound through voice effects and cleanup into a cached copy
/// with the same timing, so the timeline can use it in place of the original.
enum AudioProcessor {
    private static let inFlight = Locked<[String: Task<URL, Error>]>([:])

    static func processed(_ source: URL, effect: AudioEffect, cleanup: VoiceCleanup, media: URL) async throws -> URL {
        let folder = media.appendingPathComponent("processed", isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let name = "\(source.deletingPathExtension().lastPathComponent)-\(effect.rawValue)-\(cleanup.description).caf"
        let out = folder.appendingPathComponent(name)
        if FileManager.default.fileExists(atPath: out.path) { return out }
        let task: Task<URL, Error> = inFlight.withLock { tasks in
            if let running = tasks[name] { return running }
            let t = Task.detached(priority: .userInitiated) { () throws -> URL in
                try await render(source, effect: effect, cleanup: cleanup, to: out, scratch: folder)
                return out
            }
            tasks[name] = t
            return t
        }
        defer { _ = inFlight.withLock { $0.removeValue(forKey: name) } }
        return try await task.value
    }

    private static func render(_ source: URL, effect: AudioEffect, cleanup: VoiceCleanup, to out: URL, scratch: URL) async throws {
        // Pull the sound out of the movie first; AVAudioFile reads audio files, not video containers.
        let extracted = scratch.appendingPathComponent(UUID().uuidString + ".m4a")
        defer { try? FileManager.default.removeItem(at: extracted) }
        let asset = AVURLAsset(url: source)
        guard let session = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetAppleM4A) else {
            throw CocoaError(.featureUnsupported)
        }
        try await session.export(to: extracted, as: .m4a)

        let input = try AVAudioFile(forReading: extracted)
        let format = input.processingFormat
        let engine = AVAudioEngine()
        let player = AVAudioPlayerNode()
        engine.attach(player)
        var chain: [AVAudioNode] = [player]
        func add(_ node: AVAudioNode) { engine.attach(node); chain.append(node) }

        if cleanup.denoise {
            let desc = AudioComponentDescription(componentType: kAudioUnitType_Effect, componentSubType: kAudioUnitSubType_AUSoundIsolation,
                                                 componentManufacturer: kAudioUnitManufacturer_Apple, componentFlags: 0, componentFlagsMask: 0)
            if AudioComponentFindNext(nil, [desc]) != nil {
                add(AVAudioUnitEffect(audioComponentDescription: desc))
            } else {
                add(eq([(.highPass, 120, 0, 0.7), (.lowPass, 9000, 0, 0.7)]))
            }
        }
        if cleanup.enhance {
            add(eq([(.highPass, 90, 0, 0.7), (.parametric, 250, -3, 1), (.parametric, 3200, 4, 0.8), (.highShelf, 9000, 2, 0.7)]))
            let desc = AudioComponentDescription(componentType: kAudioUnitType_Effect, componentSubType: kAudioUnitSubType_DynamicsProcessor,
                                                 componentManufacturer: kAudioUnitManufacturer_Apple, componentFlags: 0, componentFlagsMask: 0)
            add(AVAudioUnitEffect(audioComponentDescription: desc))
        }
        if cleanup.pitch != 0 { add(pitch(Float(cleanup.pitch * 100))) }
        for node in nodes(for: effect) { add(node) }
        engine.attach(engine.mainMixerNode)
        for (a, b) in zip(chain, chain.dropFirst()) { engine.connect(a, to: b, format: format) }
        engine.connect(chain.last!, to: engine.mainMixerNode, format: format)

        try engine.enableManualRenderingMode(.offline, format: format, maximumFrameCount: 4096)
        try engine.start()
        player.scheduleFile(input, at: nil, completionHandler: nil)
        player.play()

        let output = try AVAudioFile(forWriting: out, settings: engine.manualRenderingFormat.settings,
                                     commonFormat: .pcmFormatFloat32, interleaved: false)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: engine.manualRenderingFormat, frameCapacity: 4096) else { return }
        var peak: Float = 0
        var sumSquares = 0.0
        var count = 0
        while engine.manualRenderingSampleTime < input.length {
            let frames = AVAudioFrameCount(min(Int64(buffer.frameCapacity), input.length - engine.manualRenderingSampleTime))
            let status = try engine.renderOffline(frames, to: buffer)
            guard status == .success else { break }
            if cleanup.normalize, let data = buffer.floatChannelData {
                for c in 0..<Int(buffer.format.channelCount) {
                    for i in 0..<Int(buffer.frameLength) {
                        let v = data[c][i]
                        peak = max(peak, abs(v)); sumSquares += Double(v * v); count += 1
                    }
                }
            }
            try output.write(from: buffer)
        }
        player.stop(); engine.stop()

        if cleanup.normalize, count > 0, peak > 0 {
            // Aim for speech-like loudness without clipping peaks.
            let rms = sqrt(sumSquares / Double(count))
            let gain = Float(min(0.1 / max(rms, 1e-6), 0.95 / Double(peak)))
            try applyGain(gain, to: out)
        }
    }

    private static func applyGain(_ gain: Float, to url: URL) throws {
        let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)) else { return }
        try file.read(into: buffer)
        if let data = buffer.floatChannelData {
            for c in 0..<Int(buffer.format.channelCount) {
                for i in 0..<Int(buffer.frameLength) { data[c][i] = max(-1, min(1, data[c][i] * gain)) }
            }
        }
        let tmp = url.deletingLastPathComponent().appendingPathComponent(UUID().uuidString + ".caf")
        let out = try AVAudioFile(forWriting: tmp, settings: file.fileFormat.settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        try out.write(from: buffer)
        _ = try FileManager.default.replaceItemAt(url, withItemAt: tmp)
    }

    private static func eq(_ bands: [(AVAudioUnitEQFilterType, Float, Float, Float)]) -> AVAudioUnitEQ {
        let unit = AVAudioUnitEQ(numberOfBands: bands.count)
        for (i, b) in bands.enumerated() {
            let band = unit.bands[i]
            band.filterType = b.0; band.frequency = b.1; band.gain = b.2; band.bandwidth = b.3; band.bypass = false
        }
        return unit
    }

    private static func reverb(_ preset: AVAudioUnitReverbPreset, _ wet: Float) -> AVAudioUnitReverb {
        let r = AVAudioUnitReverb(); r.loadFactoryPreset(preset); r.wetDryMix = wet; return r
    }

    private static func distortion(_ preset: AVAudioUnitDistortionPreset, _ wet: Float) -> AVAudioUnitDistortion {
        let d = AVAudioUnitDistortion(); d.loadFactoryPreset(preset); d.wetDryMix = wet; return d
    }

    private static func pitch(_ cents: Float) -> AVAudioUnitTimePitch {
        let p = AVAudioUnitTimePitch(); p.pitch = cents; return p
    }

    private static func nodes(for effect: AudioEffect) -> [AVAudioNode] {
        switch effect {
        case .none: return []
        case .echo:
            let d = AVAudioUnitDelay(); d.delayTime = 0.28; d.feedback = 40; d.wetDryMix = 35; return [d]
        case .room: return [reverb(.mediumRoom, 30)]
        case .hall: return [reverb(.largeHall, 40)]
        case .cathedral: return [reverb(.cathedral, 55)]
        case .stadium: return [reverb(.largeHall2, 55)]
        case .plate: return [reverb(.plate, 35)]
        case .underwater: return [pitch(-150), eq([(.lowPass, 600, 0, 0.7)]), reverb(.largeChamber, 35)]
        case .bass: return [eq([(.lowShelf, 140, 9, 0.7)])]
        case .treble: return [eq([(.highShelf, 5000, 9, 0.7)])]
        case .bright: return [eq([(.highShelf, 3000, 5, 0.7), (.parametric, 300, -2, 1)])]
        case .distorted: return [distortion(.multiDistortedSquared, 45)]
        case .lofi: return [eq([(.highPass, 250, 0, 0.7), (.lowPass, 4500, 0, 0.7)]), distortion(.multiDecimated2, 25)]
        case .radio: return [eq([(.bandPass, 1800, 0, 2)]), distortion(.speechRadioTower, 35)]
        case .telephone: return [eq([(.highPass, 400, 0, 0.7), (.lowPass, 3200, 0, 0.7)]), distortion(.multiCellphoneConcert, 25)]
        case .vinyl: return [eq([(.highPass, 150, 0, 0.7), (.lowPass, 7000, 0, 0.7)]), distortion(.multiDecimated1, 20)]
        case .chipmunk: return [pitch(700)]
        case .deep: return [pitch(-450)]
        case .monster: return [pitch(-900), distortion(.speechAlienChatter, 20)]
        case .megaphone: return [eq([(.bandPass, 1500, 6, 1.5)]), distortion(.multiDistortedCubed, 30)]
        case .robot: return [distortion(.speechCosmicInterference, 55), eq([(.parametric, 1000, 3, 1)])]
        case .sweet: return [pitch(250), eq([(.highShelf, 6000, 3, 0.7)])]
        case .whisper: return [eq([(.highPass, 1200, 0, 0.7), (.highShelf, 6000, 4, 0.7)]), reverb(.smallRoom, 20)]
        }
    }
}

/// Speaks text into an audio file with the device's voices, including a Personal Voice.
final class SpeechMaker: NSObject, @unchecked Sendable {
    private let synthesizer = AVSpeechSynthesizer()

    static func voices(language: String) -> [AVSpeechSynthesisVoice] {
        AVSpeechSynthesisVoice.speechVoices()
            .filter { $0.language.hasPrefix(language) || $0.voiceTraits.contains(.isPersonalVoice) }
            .sorted { ($0.quality.rawValue, $0.name) > ($1.quality.rawValue, $1.name) }
    }

    func write(_ text: String, voice: AVSpeechSynthesisVoice?, language: String, rate: Float, pitch: Float, to url: URL) async throws {
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = voice ?? AVSpeechSynthesisVoice(language: language)
        utterance.rate = rate
        utterance.pitchMultiplier = pitch
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            var file: AVAudioFile?
            var finished = false
            synthesizer.write(utterance) { buffer in
                guard !finished else { return }
                guard let pcm = buffer as? AVAudioPCMBuffer else { return }
                if pcm.frameLength == 0 {
                    finished = true
                    file == nil ? c.resume(throwing: CocoaError(.fileWriteUnknown)) : c.resume()
                    return
                }
                do {
                    if file == nil {
                        file = try AVAudioFile(forWriting: url, settings: pcm.format.settings,
                                               commonFormat: pcm.format.commonFormat, interleaved: pcm.format.isInterleaved)
                    }
                    try file?.write(from: pcm)
                } catch {
                    finished = true
                    c.resume(throwing: error)
                }
            }
        }
    }
}
