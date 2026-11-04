import AVFoundation
import Observation
import UIKit

/// Filmstrip frames for the timeline, made once per source and reused.
@MainActor
@Observable
final class Thumbnails {
    static let shared = Thumbnails()
    /// Keyed by file name; frames sampled every `step` source seconds.
    private(set) var frames: [String: [UIImage]] = [:]
    @ObservationIgnored private var pending: Set<String> = []
    static let step = 1.0

    func strip(for clip: Clip, projectID: UUID) -> [UIImage] {
        if let hit = frames[clip.file] { return hit }
        load(file: clip.file, projectID: projectID, duration: clip.sourceDuration)
        return []
    }

    /// Frame for source second `t`.
    func frame(_ file: String, at t: Double) -> UIImage? {
        guard let all = frames[file], !all.isEmpty else { return nil }
        return all[min(all.count - 1, max(0, Int(t / Self.step)))]
    }

    private func load(file: String, projectID: UUID, duration: Double) {
        guard !pending.contains(file) else { return }
        pending.insert(file)
        let url = ProjectStore.mediaFolder(for: projectID).appendingPathComponent(file)
        Task {
            let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
            generator.appliesPreferredTrackTransform = true
            generator.maximumSize = CGSize(width: 160, height: 160)
            generator.requestedTimeToleranceBefore = CMTime(seconds: 0.5, preferredTimescale: 600)
            generator.requestedTimeToleranceAfter = CMTime(seconds: 0.5, preferredTimescale: 600)
            let count = max(1, Int(ceil(duration / Self.step)))
            let times = (0..<count).map { CMTime(seconds: Double($0) * Self.step, preferredTimescale: 600) }
            var images: [UIImage] = []
            for await result in generator.images(for: times) {
                if let cg = try? result.image { images.append(UIImage(cgImage: cg)) }
            }
            frames[file] = images
        }
    }
}

/// Loudness peaks for the timeline's audio rows, read once per file in the background.
@MainActor
@Observable
final class Waveforms {
    static let shared = Waveforms()
    /// Peaks per second of source.
    nonisolated static let rate = 20.0
    private(set) var cache: [String: [Float]] = [:]
    @ObservationIgnored private var pending: Set<String> = []

    func peaks(for file: String, projectID: UUID) -> [Float]? {
        if let hit = cache[file] { return hit }
        guard !pending.contains(file) else { return nil }
        pending.insert(file)
        let url = ProjectStore.mediaFolder(for: projectID).appendingPathComponent(file)
        Task.detached(priority: .utility) {
            let result = (try? await Self.read(url)) ?? []
            await MainActor.run { Waveforms.shared.cache[file] = result }
        }
        return nil
    }

    /// Mono 8 kHz samples, the loudest of each 1/20 s, scaled so the loudest is 1.
    nonisolated private static func read(_ url: URL) async throws -> [Float] {
        let asset = AVURLAsset(url: url)
        guard let track = try await asset.loadTracks(withMediaType: .audio).first else { return [] }
        let reader = try AVAssetReader(asset: asset)
        let sampleRate = 8000.0
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
            AVNumberOfChannelsKey: 1,
            AVSampleRateKey: sampleRate,
        ])
        reader.add(output)
        guard reader.startReading() else { return [] }
        let bucket = Int(sampleRate / rate)
        var out: [Float] = []
        var peak: Int16 = 0
        var count = 0
        while let buffer = output.copyNextSampleBuffer() {
            guard let block = CMSampleBufferGetDataBuffer(buffer) else { continue }
            let length = CMBlockBufferGetDataLength(block)
            var samples = [Int16](repeating: 0, count: length / 2)
            samples.withUnsafeMutableBytes { raw in
                _ = CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: raw.baseAddress!)
            }
            for s in samples {
                let a = s == Int16.min ? Int16.max : abs(s)
                if a > peak { peak = a }
                count += 1
                if count == bucket {
                    out.append(Float(peak))
                    peak = 0
                    count = 0
                }
            }
        }
        if count > 0 { out.append(Float(peak)) }
        let top = max(1, out.max() ?? 1)
        return out.map { $0 / top }
    }
}
