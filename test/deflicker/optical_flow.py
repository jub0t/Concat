#!/usr/bin/env python3

import sys
import cv2
import numpy as np


ANALYSIS_W = 320

SPATIAL_SIGMA = 18.0

MIN_GAIN = 0.60
MAX_GAIN = 2.20

CONFIDENCE_THRESHOLD = 0.20


def luminance(frame):
    f = frame.astype(np.float32) / 255.0

    return (
        0.0722 * f[:, :, 0] +
        0.7152 * f[:, :, 1] +
        0.2126 * f[:, :, 2]
    )


def flow_warp(source, target):
    """
    Warp SOURCE toward TARGET.

    Flow is calculated from target -> source so that
    remap tells us where each target pixel came from.
    """

    target8 = np.clip(
        target * 255,
        0,
        255
    ).astype(np.uint8)

    source8 = np.clip(
        source * 255,
        0,
        255
    ).astype(np.uint8)

    flow = cv2.calcOpticalFlowFarneback(
        target8,
        source8,
        None,
        0.5,
        4,
        21,
        5,
        7,
        1.5,
        0
    )

    h, w = target.shape

    x, y = np.meshgrid(
        np.arange(w),
        np.arange(h)
    )

    map_x = (
        x.astype(np.float32) +
        flow[:, :, 0]
    )

    map_y = (
        y.astype(np.float32) +
        flow[:, :, 1]
    )

    warped = cv2.remap(
        source,
        map_x,
        map_y,
        cv2.INTER_LINEAR,
        borderMode=cv2.BORDER_REFLECT
    )

    return warped


def main():
    if len(sys.argv) != 3:
        print(
            "Usage: python optical_flow.py "
            "input.mp4 output.mp4"
        )
        sys.exit(1)

    src, dst = sys.argv[1], sys.argv[2]

    cap = cv2.VideoCapture(src)

    if not cap.isOpened():
        raise RuntimeError(f"Cannot open {src}")

    fps = cap.get(cv2.CAP_PROP_FPS)
    width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    count = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))

    analysis_h = max(
        96,
        round(
            ANALYSIS_W *
            height /
            width
        )
    )

    print(
        f"Input: {width}x{height} "
        f"@ {fps:.3f} fps"
    )

    print(f"Frames: {count}")

    print(
        f"Flow analysis: "
        f"{ANALYSIS_W}x{analysis_h}"
    )

    frames = []
    lums = []

    print()
    print("Reading frames...")

    while True:

        ok, frame = cap.read()

        if not ok:
            break

        frames.append(frame)

        small = cv2.resize(
            frame,
            (ANALYSIS_W, analysis_h),
            interpolation=cv2.INTER_AREA
        )

        lums.append(
            luminance(small)
        )

    cap.release()

    n = len(frames)

    writer = cv2.VideoWriter(
        dst,
        cv2.VideoWriter_fourcc(*"mp4v"),
        fps,
        (width, height)
    )

    print()
    print("Motion-compensated correction...")

    for i in range(n):

        current = lums[i]

        ratio_maps = []

        neighbors = []

        if i > 0:
            neighbors.append(i - 1)

        if i + 1 < n:
            neighbors.append(i + 1)

        for j in neighbors:

            warped = flow_warp(
                lums[j],
                current
            )

            # ------------------------------------------------
            # Estimate multiplicative illumination difference.
            # ------------------------------------------------

            ratio = (
                warped + 0.02
            ) / (
                current + 0.02
            )

            # ------------------------------------------------
            # Reject pixels where correspondence is obviously
            # suspicious.
            # ------------------------------------------------

            residual = np.abs(
                warped - current
            )

            confidence = np.exp(
                -residual / 0.12
            )

            valid = (
                confidence >
                CONFIDENCE_THRESHOLD
            )

            ratio = np.where(
                valid,
                ratio,
                1.0
            )

            ratio_maps.append(
                ratio
            )

        if not ratio_maps:

            writer.write(frames[i])
            continue

        ratios = np.stack(
            ratio_maps
        )

        # Robust consensus between previous/next frames.
        illumination = np.median(
            ratios,
            axis=0
        )

        # ----------------------------------------------------
        # Illumination should be spatially smooth.
        #
        # This is crucial: we don't want to "correct" textures,
        # skateboarder details, etc.
        # ----------------------------------------------------

        illumination = cv2.GaussianBlur(
            illumination,
            (0, 0),
            SPATIAL_SIGMA
        )

        illumination = np.clip(
            illumination,
            MIN_GAIN,
            MAX_GAIN
        )

        # ----------------------------------------------------
        # We only want to remove rapid deviation.
        #
        # Split the difference because neighbors may themselves
        # sit on opposite phases of the flicker.
        # ----------------------------------------------------

        gain = np.sqrt(
            np.maximum(
                illumination,
                0.01
            )
        )

        gain = np.clip(
            gain,
            MIN_GAIN,
            MAX_GAIN
        )

        gain_full = cv2.resize(
            gain,
            (width, height),
            interpolation=cv2.INTER_CUBIC
        )

        corrected = (
            frames[i].astype(np.float32) *
            gain_full[:, :, None]
        )

        corrected = np.clip(
            corrected,
            0,
            255
        ).astype(np.uint8)

        writer.write(
            corrected
        )

        if (i + 1) % 10 == 0:

            print(
                f"\r{i + 1}/{n}",
                end="",
                flush=True
            )

    writer.release()

    print()
    print(f"Done: {dst}")


if __name__ == "__main__":
    main()