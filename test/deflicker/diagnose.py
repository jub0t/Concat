#!/usr/bin/env python3

import sys
import cv2
import numpy as np

RINGS = 20
ANALYSIS_W = 320
SECTORS = 4

SECTOR_NAMES = [
    "RIGHT",
    "BOTTOM",
    "LEFT",
    "TOP",
]


def luminance(frame):
    f = frame.astype(np.float32) / 255.0

    return (
        0.0722 * f[:, :, 0]
        + 0.7152 * f[:, :, 1]
        + 0.2126 * f[:, :, 2]
    )


def robust_median(values):
    if values.size == 0:
        return np.nan

    lo = np.percentile(values, 10)
    hi = np.percentile(values, 90)

    trimmed = values[
        (values >= lo) &
        (values <= hi)
    ]

    if trimmed.size == 0:
        return float(np.median(values))

    return float(np.median(trimmed))


def make_geometry(h, w):
    """
    Elliptical radius instead of corner-normalized Euclidean radius.

    radius = 1 at:
        left/right image edge
        top/bottom image edge

    This means every radial ring contains meaningful pixels on a
    rectangular video.
    """

    y, x = np.mgrid[0:h, 0:w].astype(np.float32)

    cx = (w - 1) * 0.5
    cy = (h - 1) * 0.5

    nx = (x - cx) / max(cx, 1.0)
    ny = (y - cy) / max(cy, 1.0)

    radius = np.sqrt(
        nx * nx +
        ny * ny
    )

    # Angle:
    #
    # 0          = right
    # pi/2       = bottom
    # +/- pi     = left
    # -pi/2      = top

    angle = np.arctan2(
        ny,
        nx
    )

    return radius, angle


def sector_masks(angle):
    """
    Divide frame into four 90-degree directional sectors.
    """

    pi = np.pi

    right = (
        (angle >= -pi / 4) &
        (angle < pi / 4)
    )

    bottom = (
        (angle >= pi / 4) &
        (angle < 3 * pi / 4)
    )

    left = (
        (angle >= 3 * pi / 4) |
        (angle < -3 * pi / 4)
    )

    top = (
        (angle >= -3 * pi / 4) &
        (angle < -pi / 4)
    )

    return [
        right,
        bottom,
        left,
        top,
    ]


def safe_std(x):
    x = x[np.isfinite(x)]

    if len(x) == 0:
        return 0.0

    return float(np.std(x))


def run_pca(X):
    """
    PCA via SVD.

    Rows = frames
    Columns = spatial measurements
    """

    X = X.copy()

    column_mean = np.nanmean(
        X,
        axis=0,
        keepdims=True
    )

    X -= column_mean

    X = np.nan_to_num(
        X,
        nan=0.0,
        posinf=0.0,
        neginf=0.0
    )

    U, S, Vt = np.linalg.svd(
        X,
        full_matrices=False
    )

    variance = S * S

    total = np.sum(variance)

    if total <= 0:
        explained = np.zeros_like(variance)
    else:
        explained = variance / total

    return explained, U, S, Vt


def correlation(a, b):
    mask = (
        np.isfinite(a) &
        np.isfinite(b)
    )

    a = a[mask]
    b = b[mask]

    if len(a) < 3:
        return 0.0

    if np.std(a) < 1e-8:
        return 0.0

    if np.std(b) < 1e-8:
        return 0.0

    return float(
        np.corrcoef(a, b)[0, 1]
    )


def main():
    if len(sys.argv) != 2:
        print(
            "Usage: python diagnose.py input.mp4"
        )
        sys.exit(1)

    path = sys.argv[1]

    cap = cv2.VideoCapture(path)

    if not cap.isOpened():
        raise RuntimeError(
            f"Cannot open {path}"
        )

    fps = cap.get(
        cv2.CAP_PROP_FPS
    )

    width = int(
        cap.get(
            cv2.CAP_PROP_FRAME_WIDTH
        )
    )

    height = int(
        cap.get(
            cv2.CAP_PROP_FRAME_HEIGHT
        )
    )

    frame_count = int(
        cap.get(
            cv2.CAP_PROP_FRAME_COUNT
        )
    )

    analysis_h = max(
        96,
        round(
            ANALYSIS_W *
            height /
            width
        )
    )

    radius, angle = make_geometry(
        analysis_h,
        ANALYSIS_W
    )

    sectors = sector_masks(
        angle
    )

    ring_edges = np.linspace(
        0.0,
        1.0,
        RINGS + 1
    )

    radial_profiles = []
    sector_profiles = []

    print()
    print(
        f"Input: {width}x{height} "
        f"@ {fps:.3f} fps"
    )

    print(
        f"Frames: {frame_count}"
    )

    print(
        f"Analysis: "
        f"{ANALYSIS_W}x{analysis_h}"
    )

    print(
        f"Radial rings: {RINGS}"
    )

    print(
        "Directional sectors: "
        "RIGHT / BOTTOM / LEFT / TOP"
    )

    print()
    print("Analyzing...")

    frame_index = 0

    while True:
        ok, frame = cap.read()

        if not ok:
            break

        small = cv2.resize(
            frame,
            (
                ANALYSIS_W,
                analysis_h
            ),
            interpolation=cv2.INTER_AREA
        )

        lum = luminance(
            small
        )

        radial = []

        directional = np.full(
            (
                SECTORS,
                RINGS
            ),
            np.nan,
            dtype=np.float32
        )

        for r in range(RINGS):

            ring_mask = (
                (radius >= ring_edges[r]) &
                (radius < ring_edges[r + 1])
            )

            radial.append(
                robust_median(
                    lum[ring_mask]
                )
            )

            for s in range(SECTORS):

                mask = (
                    ring_mask &
                    sectors[s]
                )

                directional[s, r] = (
                    robust_median(
                        lum[mask]
                    )
                )

        radial_profiles.append(
            radial
        )

        sector_profiles.append(
            directional
        )

        frame_index += 1

        if frame_index % 30 == 0:

            print(
                f"\r{frame_index}/"
                f"{frame_count}",
                end="",
                flush=True
            )

    cap.release()

    print()

    radial_profiles = np.asarray(
        radial_profiles,
        dtype=np.float32
    )

    sector_profiles = np.asarray(
        sector_profiles,
        dtype=np.float32
    )

    # ========================================================
    # Normalize using central region.
    #
    # This removes global exposure variation from the
    # diagnostic and isolates spatial changes.
    # ========================================================

    center = np.nanmedian(
        radial_profiles[:, :3],
        axis=1
    )

    center = np.maximum(
        center,
        1e-5
    )

    radial_norm = (
        radial_profiles /
        center[:, None]
    )

    sector_norm = (
        sector_profiles /
        center[:, None, None]
    )

    # ========================================================
    # Radial instability
    # ========================================================

    radial_std = np.nanstd(
        radial_norm,
        axis=0
    )

    radial_range = (
        np.nanmax(
            radial_norm,
            axis=0
        )
        -
        np.nanmin(
            radial_norm,
            axis=0
        )
    )

    print()
    print(
        "RADIAL TEMPORAL INSTABILITY"
    )

    print()
    print(
        "Ring   Radius       STD       Range"
    )

    print(
        "-------------------------------------"
    )

    for r in range(RINGS):

        print(
            f"{r:02d}     "
            f"{ring_edges[r]:.2f}-"
            f"{ring_edges[r + 1]:.2f}    "
            f"{radial_std[r]:.4f}    "
            f"{radial_range[r]:.4f}"
        )

    # ========================================================
    # Directional signals
    #
    # Use outer 30% of image.
    # ========================================================

    outer_start = int(
        RINGS * 0.70
    )

    directional_signals = []

    print()
    print(
        "OUTER-REGION TEMPORAL INSTABILITY"
    )

    print()

    for s in range(SECTORS):

        signal = np.nanmedian(
            sector_norm[
                :,
                s,
                outer_start:
            ],
            axis=1
        )

        directional_signals.append(
            signal
        )

        print(
            f"  {SECTOR_NAMES[s]:6s}: "
            f"STD {safe_std(signal):.4f}"
        )

    directional_signals = np.asarray(
        directional_signals
    )

    # ========================================================
    # Correlations
    #
    # If all directions strongly correlate, that supports a
    # coherent radial/vignette-like temporal effect.
    # ========================================================

    print()
    print(
        "DIRECTIONAL CORRELATIONS"
    )

    print()

    for a in range(SECTORS):

        for b in range(
            a + 1,
            SECTORS
        ):

            c = correlation(
                directional_signals[a],
                directional_signals[b]
            )

            print(
                f"  "
                f"{SECTOR_NAMES[a]:6s} vs "
                f"{SECTOR_NAMES[b]:6s}: "
                f"{c:+.3f}"
            )

    # ========================================================
    # Radial PCA
    # ========================================================

    radial_explained, _, _, radial_modes = (
        run_pca(
            radial_norm
        )
    )

    print()
    print(
        "RADIAL PCA"
    )

    print()

    for i in range(
        min(
            5,
            len(radial_explained)
        )
    ):

        print(
            f"  component {i + 1}: "
            f"{radial_explained[i] * 100:.2f}%"
        )

    # ========================================================
    # Full radial + directional PCA
    #
    # This tells us whether one coherent spatial mode also
    # explains directional variation.
    # ========================================================

    flattened = sector_norm.reshape(
        sector_norm.shape[0],
        -1
    )

    spatial_explained, _, _, spatial_modes = (
        run_pca(
            flattened
        )
    )

    print()
    print(
        "RADIAL + DIRECTIONAL PCA"
    )

    print()

    for i in range(
        min(
            5,
            len(spatial_explained)
        )
    ):

        print(
            f"  component {i + 1}: "
            f"{spatial_explained[i] * 100:.2f}%"
        )

    # ========================================================
    # Interpretation
    # ========================================================

    correlations = []

    for a in range(SECTORS):

        for b in range(
            a + 1,
            SECTORS
        ):

            correlations.append(
                correlation(
                    directional_signals[a],
                    directional_signals[b]
                )
            )

    median_corr = float(
        np.median(
            correlations
        )
    )

    pc1 = float(
        spatial_explained[0]
    )

    print()
    print(
        "SUMMARY"
    )

    print()
    print(
        f"Median directional correlation: "
        f"{median_corr:+.3f}"
    )

    print(
        f"Dominant spatial PCA mode: "
        f"{pc1 * 100:.2f}%"
    )

    print()

    if (
        median_corr > 0.75
        and pc1 > 0.65
    ):

        print(
            "RESULT:"
        )

        print(
            "Strong evidence for one coherent "
            "time-varying vignette/spatial effect."
        )

        print(
            "A low-dimensional parametric/PCA "
            "remover is worth building."
        )

    elif (
        median_corr > 0.45
        or pc1 > 0.50
    ):

        print(
            "RESULT:"
        )

        print(
            "There is meaningful coherent spatial "
            "flicker, but it is not purely radial."
        )

        print(
            "Use a multi-mode spatial model or "
            "blind neural deflickering."
        )

    else:

        print(
            "RESULT:"
        )

        print(
            "The artifact is not well explained "
            "by a coherent vignette-like model."
        )

        print(
            "Prioritize blind neural deflickering "
            "rather than radial correction."
        )

    # ========================================================
    # Save everything
    # ========================================================

    np.savez(
        "diagnostic_v2.npz",

        radial_profiles=radial_norm,

        sector_profiles=sector_norm,

        radial_pca_modes=radial_modes,

        spatial_pca_modes=spatial_modes,

        directional_signals=directional_signals,

        ring_edges=ring_edges,

        fps=np.float32(fps)
    )

    print()
    print(
        "Saved: diagnostic_v2.npz"
    )
    print()


if __name__ == "__main__":
    main()