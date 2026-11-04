#if DEBUG
import AVFoundation
import CoreImage
import Foundation
import UIKit

/// Exercises the features a build check can't see working, on a real Simulator:
/// Whisper captions, stabilisation, the brush mask, 10-bit HDR export, karaoke
/// captions, templates and timelines. CI launches the app with `-selftest`, waits
/// for Documents/selftest.json and fails if any entry isn't "ok". Debug builds only.
@MainActor
enum SelfTest {
    static var resultURL: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("selftest.json")
    }

    static func run(store: ProjectStore) async {
        var results: [String: String] = [:]
        func record(_ name: String, _ body: () async throws -> Void) async {
            do {
                try await body()
                results[name] = "ok"
            } catch {
                results[name] = "FAILED: \(error.localizedDescription) \(error)"
            }
            write(results, done: false)
        }

        guard let project = store.projects.first(where: { $0.name == "CI self test" }) else {
            write(["setup": "FAILED: no project named CI self test"], done: true)
            return
        }
        let clip = project.clips[0]
        let clipURL = ProjectStore.mediaFolder(for: project.id).appendingPathComponent(clip.file)

        await record("timelines") {
            var p = project
            let first = p.timelineID
            p.addTimeline(copying: true)
            try check(p.otherTimelines.count == 1 && p.clips.count == project.clips.count, "copy holds the clips")
            p.aspect = .landscape
            p.switchTimeline(to: first)
            try check(p.timelineID == first && p.aspect == project.aspect, "switching back restores the first timeline")
            try check(p.otherTimelines.first?.aspect == .landscape, "the second timeline kept its own canvas")
            p.addTimeline(copying: false)
            try check(p.clips.isEmpty && p.otherTimelines.count == 2, "an empty timeline starts empty")
        }

        await record("templates") {
            store.saveTemplate(from: project, name: "CI template")
            guard let template = store.templates.first(where: { $0.name == "CI template" }) else { throw Fail("template not saved") }
            let made = store.project(fromTemplate: template)
            try check(made.clips.count == project.clips.count, "slots carried over")
            let media = ProjectStore.mediaFolder(for: made.id).appendingPathComponent(made.clips[0].file)
            try check(FileManager.default.fileExists(atPath: media.path), "template media copied into the new project")
            var filled = made
            var other = clip
            other.file = "replacement.mp4"; other.sourceDuration = 20
            filled.clips[0].replaceMedia(with: other)
            try check(filled.clips[0].file == "replacement.mp4" && abs(filled.clips[0].duration - clip.duration) < 0.01,
                      "a filled slot keeps its length")
            store.delete(made)
            store.deleteTemplate(template)
        }

        await record("brushMask") {
            let red = CIImage(color: .red).cropped(to: CGRect(x: 0, y: 0, width: 200, height: 200))
            var c = Clip()
            c.maskStrokes = [MaskStroke(points: [0.1, 0.5, 0.9, 0.5], radius: 0.1, keep: false)]
            let out = Imaging.cutout(red, clip: c)
            let centre = alpha(of: out, at: CGPoint(x: 100, y: 100))
            let corner = alpha(of: out, at: CGPoint(x: 5, y: 5))
            try check(centre < 0.05, "erased pixels are transparent (alpha \(centre))")
            try check(corner > 0.95, "unpainted pixels stay (alpha \(corner))")
        }

        await record("karaoke") {
            var text = TextOverlay(text: "ONE TWO THREE", start: 0, duration: 3, color: .white, accent: .black, size: 0.1, style: .plain)
            text.wordStarts = [0, 1, 2]
            text.karaoke = true
            text.karaokeColor = RGBA(r: 1, g: 0, b: 0)
            var p = Project()
            p.aspect = .landscape
            p.background = .black
            p.texts = [text]
            let state = RenderState(project: p, segments: [], canvas: CGSize(width: 640, height: 360))
            let early = redness(FrameRenderer.render(sources: [:], time: 0.2, state: state))
            let late = redness(FrameRenderer.render(sources: [:], time: 2.5, state: state))
            try check(late > early * 1.5 && late > 0, "more words lit later (red \(early) -> \(late))")
        }

        await record("stabilize") {
            let path = try await Stabilizer.analyse(clipURL, duration: clip.sourceDuration) { _ in }
            try check(path.count >= Int(clip.sourceDuration * 10) * 2 - 4, "a correction every 1/10 s (\(path.count / 2) samples)")
            try check(path.allSatisfy { $0.isFinite && abs($0) < 1 }, "corrections are sensible")
            try check(path.contains { abs($0) > 0.0005 }, "the shaky test clip produced a correction")
        }

        await record("hdrExport") {
            var p = project
            p.clips = [clip]
            let url = try await Exporter.export(p, quality: .hd720, hevc: true, frameRate: 30, tenBit: true) { _ in }
            let track = try await AVURLAsset(url: url).loadTracks(withMediaType: .video).first
            guard let track, let format = try await track.load(.formatDescriptions).first else { throw Fail("no video in the export") }
            let codec = CMFormatDescriptionGetMediaSubType(format)
            try check(codec == kCMVideoCodecType_HEVC, "exported as HEVC")
            let transfer = CMFormatDescriptionGetExtension(format, extensionKey: kCMFormatDescriptionExtension_TransferFunction) as? String
            try check(transfer == (kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG as String), "HLG transfer (\(transfer ?? "none"))")
        }

        await record("whisperCaptions") {
            let captions = try await Captioner.captions(for: project, locale: "en-US", wordsPerCaption: 3, engine: .whisper, model: .tiny)
            let words = captions.map(\.text).joined(separator: " ").lowercased()
            try check(!captions.isEmpty, "captions made")
            try check(words.contains("caption") || words.contains("test") || words.contains("editor"), "heard the spoken words: \(words)")
            try check(captions.allSatisfy { !$0.wordStarts.isEmpty && $0.duration > 0.2 }, "every caption has word timings")
        }

        write(results, done: true)
    }

    private struct Fail: LocalizedError {
        let message: String
        init(_ message: String) { self.message = message }
        var errorDescription: String? { message }
    }

    private static func check(_ ok: Bool, _ what: String) throws {
        if !ok { throw Fail(what) }
    }

    private static let context = CIContext(options: [.workingColorSpace: NSNull()])

    private static func alpha(of image: CIImage, at point: CGPoint) -> Double {
        var pixel = [UInt8](repeating: 0, count: 4)
        context.render(image, toBitmap: &pixel, rowBytes: 4, bounds: CGRect(x: point.x, y: point.y, width: 1, height: 1),
                       format: .RGBA8, colorSpace: nil)
        return Double(pixel[3]) / 255
    }

    /// How much pure red is in the frame (the karaoke colour).
    private static func redness(_ image: CIImage) -> Double {
        let w = Int(image.extent.width), h = Int(image.extent.height)
        var pixels = [UInt8](repeating: 0, count: w * h * 4)
        context.render(image, toBitmap: &pixels, rowBytes: w * 4, bounds: image.extent, format: .RGBA8, colorSpace: nil)
        var count = 0
        for i in stride(from: 0, to: pixels.count, by: 4) where pixels[i] > 180 && pixels[i + 1] < 80 && pixels[i + 2] < 80 {
            count += 1
        }
        return Double(count)
    }

    private static func write(_ results: [String: String], done: Bool) {
        var out = results
        out["_done"] = done ? "yes" : "no"
        if let data = try? JSONSerialization.data(withJSONObject: out, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: resultURL, options: .atomic)
        }
    }
}
#endif
