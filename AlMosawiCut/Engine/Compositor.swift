import AVFoundation
import CoreImage

/// Carries the render state to the compositor for the whole timeline.
final class RenderInstruction: NSObject, AVVideoCompositionInstructionProtocol, @unchecked Sendable {
    let timeRange: CMTimeRange
    let enablePostProcessing = false
    let containsTweening = true
    let requiredSourceTrackIDs: [NSValue]?
    let passthroughTrackID = kCMPersistentTrackID_Invalid
    let trackIDs: [CMPersistentTrackID]
    let state: RenderState

    init(timeRange: CMTimeRange, trackIDs: [CMPersistentTrackID], state: RenderState) {
        self.timeRange = timeRange
        self.trackIDs = trackIDs
        self.state = state
        requiredSourceTrackIDs = trackIDs.map { NSNumber(value: $0) }
    }
}

/// Draws every output frame with FrameRenderer into BGRA buffers. Owning the
/// buffers and the CIContext keeps preview and export on the same path.
final class RenderCompositor: NSObject, AVVideoCompositing, @unchecked Sendable {
    private static let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
    private let context = CIContext(options: [.workingColorSpace: RenderCompositor.colorSpace, .cacheIntermediates: false])
    private let queue = DispatchQueue(label: "AlMosawiCut.render")

    var sourcePixelBufferAttributes: [String: any Sendable]? {
        [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
         kCVPixelBufferIOSurfacePropertiesKey as String: [String: String]()]
    }

    var requiredPixelBufferAttributesForRenderContext: [String: any Sendable] {
        [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
         kCVPixelBufferIOSurfacePropertiesKey as String: [String: String]()]
    }

    func renderContextChanged(_ newRenderContext: AVVideoCompositionRenderContext) {}

    func startRequest(_ request: AVAsynchronousVideoCompositionRequest) {
        queue.async { [context] in
            guard let instruction = request.videoCompositionInstruction as? RenderInstruction,
                  let output = request.renderContext.newPixelBuffer() else {
                request.finish(with: NSError(domain: "AlMosawiCut", code: 2))
                return
            }
            let size = request.renderContext.size
            var sources: [CMPersistentTrackID: CIImage] = [:]
            for id in instruction.trackIDs {
                if let buffer = request.sourceFrame(byTrackID: id) { sources[id] = CIImage(cvPixelBuffer: buffer) }
            }
            let image = FrameRenderer.render(sources: sources, time: request.compositionTime.seconds, state: instruction.state)
            context.render(image, to: output, bounds: CGRect(origin: .zero, size: size), colorSpace: Self.colorSpace)
            request.finish(withComposedVideoFrame: output)
        }
    }

    func cancelAllPendingVideoCompositionRequests() {}
}
