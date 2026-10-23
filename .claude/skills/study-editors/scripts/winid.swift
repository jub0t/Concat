// The window number of the largest on-screen, normal-layer window whose
// owner's name contains the first argument (case-insensitive).
import CoreGraphics

let wanted = CommandLine.arguments.dropFirst().first?.lowercased() ?? ""
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
var best: (id: Int, area: Double)? = nil
for window in windows {
    guard let owner = (window[kCGWindowOwnerName as String] as? String)?.lowercased(),
          owner.contains(wanted),
          (window[kCGWindowLayer as String] as? Int) == 0,
          let bounds = window[kCGWindowBounds as String] as? [String: Double],
          let id = window[kCGWindowNumber as String] as? Int
    else { continue }
    let area = (bounds["Width"] ?? 0) * (bounds["Height"] ?? 0)
    if area > (best?.area ?? 0) { best = (id, area) }
}
if let best { print(best.id) }
