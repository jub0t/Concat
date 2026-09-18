#!/usr/bin/env python3

import sys
import cv2
import numpy as np


# ============================================================
# Configuration
# ============================================================

ANALYSIS_W = 160

CENTER_RADIUS = 0.28

EDGE_RADIUS_MIN = 0.62
EDGE_RADIUS_MAX = 0.92

CORRECTION_START_RADIUS = 0.25

TARGET_PERCENTILE = 98.0

MAX_GAIN = 2.5


# ============================================================
# Image analysis
# ============================================================

def luminance(frame):
    """
    BGR uint8 -> luminance float32.
    """
    f = frame.astype(np.float32)

    b = f[:, :, 0]
    g = f[:, :, 1]
    r = f[:, :, 2]

    return (
        0.0722 * b +
        0.7152 * g +
        0.2126 * r
    )


def radial_map(h, w):
    """
    Normalized radial distance.

    center  = 0
    corners = ~1
    """
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)

    cx = (w - 1) * 0.5
    cy = (h - 1) * 0.5

    x = (x - cx) / max(cx, 1.0)
    y = (y - cy) / max(cy, 1.0)

    radius = np.sqrt(x * x + y * y)

    return radius / np.sqrt(2.0)


def robust_mean(values):
    """
    Trim bright/dark outliers before calculating mean.

    Helps prevent lamps, shadows, skateboarder, etc.
    from dominating the measurement.
    """
    if values.size == 0:
        return 0.0

    lo = np.percentile(values, 20)
    hi = np.percentile(values, 80)

    trimmed = values[
        (values >= lo) &
        (values <= hi)
    ]

    if trimmed.size == 0:
        return float(np.mean(values))

    return float(np.mean(trimmed))


def measure_vignette(frame, radius_small):
    """
    Measure edge brightness relative to center brightness.
    """

    h, w = radius_small.shape

    small = cv2.resize(
        frame,
        (w, h),
        interpolation=cv2.INTER_AREA
    )

    lum = luminance(small)

    center_mask = radius_small < CENTER_RADIUS

    edge_mask = (
        (radius_small > EDGE_RADIUS_MIN) &
        (radius_small < EDGE_RADIUS_MAX)
    )

    center = robust_mean(
        lum[center_mask]
    )

    edge = robust_mean(
        lum[edge_mask]
    )

    if center < 1.0:
        return 1.0, center, edge

    ratio = edge / center

    return ratio, center, edge


# ============================================================
# Main
# ============================================================

def main():

    if len(sys.argv) != 3:
        print()
        print("Usage:")
        print(
            "  python main.py "
            "input.mp4 output.mp4"
        )
        print()
        sys.exit(1)

    input_path = sys.argv[1]
    output_path = sys.argv[2]

    # --------------------------------------------------------
    # Open source
    # --------------------------------------------------------

    cap = cv2.VideoCapture(input_path)

    if not cap.isOpened():
        raise RuntimeError(
            f"Could not open: {input_path}"
        )

    fps = cap.get(cv2.CAP_PROP_FPS)

    width = int(
        cap.get(cv2.CAP_PROP_FRAME_WIDTH)
    )

    height = int(
        cap.get(cv2.CAP_PROP_FRAME_HEIGHT)
    )

    frame_count = int(
        cap.get(cv2.CAP_PROP_FRAME_COUNT)
    )

    print()
    print(
        f"Input: {width}x{height} "
        f"@ {fps:.3f} fps"
    )

    print(
        f"Frames: {frame_count}"
    )

    # ========================================================
    # PASS 1
    #
    # Analyze the brightness relationship between the
    # center and edges.
    #
    # This operates at very low resolution because we're
    # measuring low-frequency illumination, not detail.
    # ========================================================

    analysis_h = max(
        64,
        round(
            ANALYSIS_W *
            height /
            width
        )
    )

    analysis_radius = radial_map(
        analysis_h,
        ANALYSIS_W
    )

    ratios = []
    centers = []
    edges = []

    print()
    print("Analyzing vignette...")

    while True:

        ok, frame = cap.read()

        if not ok:
            break

        ratio, center, edge = measure_vignette(
            frame,
            analysis_radius
        )

        ratios.append(ratio)
        centers.append(center)
        edges.append(edge)

    cap.release()

    ratios = np.asarray(
        ratios,
        dtype=np.float32
    )

    if len(ratios) == 0:
        raise RuntimeError(
            "No video frames decoded."
        )

    # --------------------------------------------------------
    # Determine what an almost-unvignetted frame looks like.
    #
    # We use the 98th percentile instead of maximum because
    # one abnormal frame shouldn't determine the target.
    # --------------------------------------------------------

    target_ratio = float(
        np.percentile(
            ratios,
            TARGET_PERCENTILE
        )
    )

    print()
    print("Edge/center ratio:")
    print(
        f"  min:     "
        f"{ratios.min():.4f}"
    )
    print(
        f"  median:  "
        f"{np.median(ratios):.4f}"
    )
    print(
        f"  max:     "
        f"{ratios.max():.4f}"
    )
    print(
        f"  target:  "
        f"{target_ratio:.4f}"
    )

    # --------------------------------------------------------
    # IMPORTANT:
    #
    # No temporal smoothing.
    #
    # The unwanted vignette itself changes rapidly, so we
    # want to react to each individual frame.
    # --------------------------------------------------------

    measured = ratios

    # --------------------------------------------------------
    # Calculate how much brighter the edge needs to become.
    #
    # Example:
    #
    # target = 0.80
    # frame  = 0.40
    #
    # gain = 0.80 / 0.40
    #      = 2.0
    # --------------------------------------------------------

    required_edge_gain = (
        target_ratio /
        np.maximum(
            measured,
            0.10
        )
    )

    required_edge_gain = np.clip(
        required_edge_gain,
        1.0,
        MAX_GAIN
    )

    print()
    print(
        "Correction gain:"
    )

    print(
        f"  min: "
        f"{required_edge_gain.min():.3f}x"
    )

    print(
        f"  max: "
        f"{required_edge_gain.max():.3f}x"
    )

    # ========================================================
    # Build full-resolution radial correction weighting.
    # ========================================================

    full_radius = radial_map(
        height,
        width
    )

    radial_weight = (
        full_radius -
        CORRECTION_START_RADIUS
    ) / (
        1.0 -
        CORRECTION_START_RADIUS
    )

    radial_weight = np.clip(
        radial_weight,
        0.0,
        1.0
    )

    # --------------------------------------------------------
    # Smoothstep
    #
    # x²(3 - 2x)
    #
    # Gives us a smooth transition from the untouched center
    # to the corrected edges.
    # --------------------------------------------------------

    radial_weight = (
        radial_weight *
        radial_weight *
        (
            3.0 -
            2.0 * radial_weight
        )
    )

    # ========================================================
    # PASS 2
    #
    # Apply per-frame correction.
    # ========================================================

    cap = cv2.VideoCapture(
        input_path
    )

    if not cap.isOpened():
        raise RuntimeError(
            "Could not reopen input video."
        )

    fourcc = cv2.VideoWriter_fourcc(
        *"mp4v"
    )

    writer = cv2.VideoWriter(
        output_path,
        fourcc,
        fps,
        (
            width,
            height
        )
    )

    if not writer.isOpened():
        raise RuntimeError(
            "Could not create output video."
        )

    print()
    print("Correcting frames...")

    frame_index = 0

    while True:

        ok, frame = cap.read()

        if not ok:
            break

        edge_gain = required_edge_gain[
            min(
                frame_index,
                len(required_edge_gain) - 1
            )
        ]

        # ----------------------------------------------------
        # Construct gain map.
        #
        # Center:
        #
        #     gain = 1.0
        #
        # Edge:
        #
        #     gain = calculated edge correction
        # ----------------------------------------------------

        gain_map = (
            1.0 +
            radial_weight *
            (
                edge_gain -
                1.0
            )
        )

        corrected = frame.astype(
            np.float32
        )

        corrected *= gain_map[
            :, :, None
        ]

        corrected = np.clip(
            corrected,
            0.0,
            255.0
        )

        corrected = corrected.astype(
            np.uint8
        )

        writer.write(
            corrected
        )

        frame_index += 1

        if frame_index % 30 == 0:

            print(
                f"\r"
                f"{frame_index}/"
                f"{len(ratios)} frames",
                end="",
                flush=True
            )

    print()

    cap.release()
    writer.release()

    print()
    print(
        f"Done: {output_path}"
    )
    print()


if __name__ == "__main__":
    main()