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
