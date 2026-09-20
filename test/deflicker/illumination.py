#!/usr/bin/env python3

import sys
import cv2
import numpy as np


ANALYSIS_W = 128
BLUR_SIGMA = 10.0
MAX_GAIN = 2.5
MIN_GAIN = 0.60
REFERENCE_PERCENTILE = 90


def luminance(frame):
    f = frame.astype(np.float32) / 255.0
    return (
        0.0722 * f[:, :, 0] +
        0.7152 * f[:, :, 1] +
        0.2126 * f[:, :, 2]
    )


def main():
    if len(sys.argv) != 3:
        print("Usage: python illumination.py input.mp4 output.mp4")
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
        64,
        round(ANALYSIS_W * height / width)
    )

    print(f"Input: {width}x{height} @ {fps:.3f} fps")
    print(f"Frames: {count}")
    print(f"Analysis: {ANALYSIS_W}x{analysis_h}")
    print()
    print("Reading frames...")

    small_lums = []
    frames = []

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

        lum = luminance(small)

        # Remove high-frequency image detail.
        lum = cv2.GaussianBlur(
            lum,
            (0, 0),
            BLUR_SIGMA
        )

        small_lums.append(lum)

    cap.release()

    lums = np.stack(small_lums)

    print("Building temporal reference...")

    # Bright temporal reference helps reconstruct areas
    # temporarily darkened by the flickering effect.
    reference = np.percentile(
        lums,
        REFERENCE_PERCENTILE,
        axis=0
    ).astype(np.float32)

    reference = cv2.GaussianBlur(
        reference,
        (0, 0),
        BLUR_SIGMA
    )

    print("Correcting frames...")

    fourcc = cv2.VideoWriter_fourcc(*"mp4v")

    writer = cv2.VideoWriter(
        dst,
        fourcc,
        fps,
        (width, height)
    )

    for i, frame in enumerate(frames):

        current = lums[i]

        gain = reference / np.maximum(current, 0.03)

        gain = np.clip(
            gain,
            MIN_GAIN,
            MAX_GAIN
        )

        # Smooth the gain field itself.
        gain = cv2.GaussianBlur(
            gain,
            (0, 0),
            5.0
        )

        gain_full = cv2.resize(
            gain,
            (width, height),
            interpolation=cv2.INTER_CUBIC
        )

        corrected = (
            frame.astype(np.float32) *
            gain_full[:, :, None]
        )

        corrected = np.clip(
            corrected,
            0,
            255
        ).astype(np.uint8)

        writer.write(corrected)

        if (i + 1) % 30 == 0:
            print(
                f"\r{i + 1}/{len(frames)}",
                end="",
                flush=True
            )

    writer.release()

    print()
    print(f"Done: {dst}")


if __name__ == "__main__":
    main()