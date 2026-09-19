#!/usr/bin/env python3

import sys
import cv2
import numpy as np


GRID_X = 8
GRID_Y = 5

MIN_FLICKER_HZ = 3.0
MAX_FLICKER_HZ = 14.0

SUPPRESSION = 0.90

MIN_GAIN = 0.65
MAX_GAIN = 1.60


def luminance(frame):
    f = frame.astype(np.float32) / 255.0

    return (
        0.0722 * f[:, :, 0] +
        0.7152 * f[:, :, 1] +
        0.2126 * f[:, :, 2]
    )


def main():
    if len(sys.argv) != 3:
        print("Usage: python fft_pwm.py input.mp4 output.mp4")
        sys.exit(1)

    src, dst = sys.argv[1], sys.argv[2]

    cap = cv2.VideoCapture(src)

    if not cap.isOpened():
        raise RuntimeError(f"Cannot open {src}")

    fps = cap.get(cv2.CAP_PROP_FPS)
    width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH))
    height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
    count = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))

    print(f"Input: {width}x{height} @ {fps:.3f} fps")
    print(f"Frames: {count}")
    print(f"Grid: {GRID_X}x{GRID_Y}")
    print()

    frames = []
    signals = []

    print("Analyzing regional brightness...")

    while True:
        ok, frame = cap.read()

        if not ok:
            break

        frames.append(frame)

        lum = luminance(frame)

        # Downsample directly into regional brightness cells.
        grid = cv2.resize(
            lum,
            (GRID_X, GRID_Y),
            interpolation=cv2.INTER_AREA
        )

        signals.append(grid)

    cap.release()

    signals = np.stack(signals)

    n = signals.shape[0]

    corrected_signals = np.zeros_like(signals)

    frequencies = np.fft.rfftfreq(
        n,
        d=1.0 / fps
    )

    flicker_band = (
        (frequencies >= MIN_FLICKER_HZ) &
        (frequencies <= MAX_FLICKER_HZ)
    )

    print("Filtering temporal frequencies...")

    detected_peaks = []

    for y in range(GRID_Y):
        for x in range(GRID_X):

            signal = signals[:, y, x]

            spectrum = np.fft.rfft(signal)

            magnitude = np.abs(spectrum)

            band_indices = np.where(
                flicker_band
            )[0]

            if len(band_indices):
                strongest = band_indices[
                    np.argmax(magnitude[band_indices])
                ]

                detected_peaks.append(
                    frequencies[strongest]
                )

            spectrum[flicker_band] *= (
                1.0 - SUPPRESSION
            )

            corrected = np.fft.irfft(
                spectrum,
                n=n
            )

            corrected_signals[:, y, x] = corrected

    if detected_peaks:
        print(
            "Median strongest frequency: "
            f"{np.median(detected_peaks):.2f} Hz"
        )

    print("Correcting frames...")

    writer = cv2.VideoWriter(
        dst,
        cv2.VideoWriter_fourcc(*"mp4v"),
        fps,
        (width, height)
    )

    for i, frame in enumerate(frames):

        original = signals[i]
        target = corrected_signals[i]

        gain = target / np.maximum(
            original,
            0.03
        )

        gain = np.clip(
            gain,
            MIN_GAIN,
            MAX_GAIN
        )

        gain = cv2.GaussianBlur(
            gain,
            (0, 0),
            1.2
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
                f"\r{i + 1}/{n}",
                end="",
                flush=True
            )

    writer.release()

    print()
    print(f"Done: {dst}")


if __name__ == "__main__":
    main()