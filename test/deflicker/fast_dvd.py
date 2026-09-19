#!/usr/bin/env python3

import argparse
import subprocess
import tempfile
import shutil
from pathlib import Path


def run(cmd):
    print("+", " ".join(map(str, cmd)))
    subprocess.run(cmd, check=True)


def main():
    parser = argparse.ArgumentParser(
        description="Quick FastDVDnet experiment on a real video."
    )
    parser.add_argument("input", help="Input video")
    parser.add_argument("output", help="Output video")
    parser.add_argument(
        "--sigma",
        type=float,
        default=15,
        help="FastDVDnet noise sigma. Try 5, 10, 15, 25."
    )
    parser.add_argument(
        "--cpu",
        action="store_true",
        help="Force CPU instead of CUDA/MPS."
    )
    args = parser.parse_args()

    input_video = Path(args.input).resolve()
    output_video = Path(args.output).resolve()

    if not input_video.exists():
        raise FileNotFoundError(input_video)

    repo = Path(__file__).resolve().parent

    if not (repo / "test_fastdvdnet.py").exists():
        raise RuntimeError(
            "Put this script in the FastDVDnet repository root."
        )

    work = Path(tempfile.mkdtemp(prefix="fastdvdnet_"))

    frames = work / "frames"
    processed = work / "processed"

    frames.mkdir()
    processed.mkdir()

    try:
        # ---------------------------------------------------------
        # 1. Get original FPS
        # ---------------------------------------------------------

        probe = subprocess.run(
            [
                "ffprobe",
                "-v", "error",
                "-select_streams", "v:0",
                "-show_entries", "stream=r_frame_rate",
                "-of", "default=noprint_wrappers=1:nokey=1",
                str(input_video)
            ],
            capture_output=True,
            text=True,
            check=True
        )

        ratio = probe.stdout.strip().split("/")

        if len(ratio) == 2:
            fps = float(ratio[0]) / float(ratio[1])
        else:
            fps = float(ratio[0])

        print(f"\nInput FPS: {fps:.3f}")
        print(f"FastDVDnet sigma: {args.sigma}")

        # ---------------------------------------------------------
        # 2. Decode video losslessly to PNG
        # ---------------------------------------------------------

        run([
            "ffmpeg",
            "-y",
            "-i", str(input_video),
            "-vsync", "0",
            str(frames / "%08d.png")
        ])

        # ---------------------------------------------------------
        # 3. Run FastDVDnet
        # ---------------------------------------------------------

        cmd = [
            "python",
            str(repo / "test_fastdvdnet.py"),
            "--test_path", str(frames),
            "--noise_sigma", str(args.sigma),
            "--save_path", str(processed),
            "--model_file", str(repo / "model.pth"),
        ]

        if args.cpu:
            cmd.append("--no_gpu")

        run(cmd)

        # ---------------------------------------------------------
        # Find where FastDVDnet actually put its output
        # ---------------------------------------------------------

        output_frames = sorted(processed.rglob("*.png"))

        if not output_frames:
            raise RuntimeError(
                f"FastDVDnet produced no PNG frames in {processed}"
            )

        # Normalize filenames because the repo can create
        # its own directory structure.
        normalized = work / "normalized"
        normalized.mkdir()

        for i, frame in enumerate(output_frames, start=1):
            shutil.copy2(
                frame,
                normalized / f"{i:08d}.png"
            )

        print(f"\nProcessed {len(output_frames)} frames.")

        # ---------------------------------------------------------
        # 4. Encode output
        # ---------------------------------------------------------

        run([
            "ffmpeg",
            "-y",
            "-framerate", str(fps),
            "-i", str(normalized / "%08d.png"),
            "-i", str(input_video),
            "-map", "0:v:0",
            "-map", "1:a?",
            "-c:v", "libx264",
            "-crf", "16",
            "-preset", "medium",
            "-pix_fmt", "yuv420p",
            "-c:a", "copy",
            "-shortest",
            str(output_video)
        ])

        print("\n================================")
        print("DONE")
        print("================================")
        print(f"Input : {input_video}")
        print(f"Output: {output_video}")
        print(f"Sigma : {args.sigma}")
        print()

    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()