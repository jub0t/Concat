#!/usr/bin/env python3
"""Apply the "Sweet" voice effect using a local FFmpeg install."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path


def probe(path: Path) -> dict:
    command = [
        "ffprobe", "-v", "error", "-show_streams", "-of", "json", str(path)
    ]
    try:
        return json.loads(subprocess.check_output(command, text=True))
    except subprocess.CalledProcessError as exc:
        raise RuntimeError(f"ffprobe could not read {path}") from exc


def sweet_filter(amount: float, pitch: float | None) -> str:
    strength = max(0.0, min(100.0, amount)) / 100.0
    semitones = pitch if pitch is not None else 1.2 + (1.5 * strength)
    ratio = 2.0 ** (semitones / 12.0)

    presence = 0.8 + (1.5 * strength)
    air = 1.0 + (2.0 * strength)
    deess = 0.18 + (0.22 * strength)
    echo = 0.012 + (0.018 * strength)

    # Raising the sample rate shifts pitch and formants; atempo restores duration.
    return ",".join([
        "aresample=48000",
        f"asetrate={48000.0 * ratio:.6f}",
        "aresample=48000",
        f"atempo={1.0 / ratio:.8f}",
        "highpass=f=85",
        "equalizer=f=300:t=q:w=1.1:g=-1.4",
        f"equalizer=f=4200:t=q:w=0.8:g={presence:.3f}",
        f"equalizer=f=10500:t=q:w=0.7:g={air:.3f}",
        "compand=attacks=0.010:decays=0.180:points=-80/-80|-30/-24|-18/-13|-8/-5|0/-1.5:soft-knee=5:gain=1",
        f"deesser=i={deess:.3f}:m=0.45:f=0.55",
        f"aecho=0.8:0.82:38:{echo:.4f}",
        "alimiter=limit=0.94:attack=5:release=60",
    ])


def output_audio_codec(path: Path) -> list[str]:
    suffix = path.suffix.lower()
    if suffix == ".wav":
        return ["-c:a", "pcm_s24le"]
    if suffix == ".flac":
        return ["-c:a", "flac"]
    if suffix == ".mp3":
        return ["-c:a", "libmp3lame", "-q:a", "2"]
    if suffix in {".webm", ".ogg", ".opus"}:
        return ["-c:a", "libopus", "-b:a", "192k"]
    return ["-c:a", "aac", "-b:a", "256k"]


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Apply the bright, soft, slightly higher Sweet voice effect."
    )
    parser.add_argument("input", type=Path, help="Input audio or video file")
    parser.add_argument("output", type=Path, nargs="?", help="Output path")
    parser.add_argument("--amount", type=float, default=65, metavar="0-100")
    parser.add_argument("--pitch", type=float, help="Pitch shift in semitones")
    parser.add_argument("--overwrite", action="store_true")
    parser.add_argument("--dry-run", action="store_true", help="Print FFmpeg command only")
    args = parser.parse_args()

    if not shutil.which("ffmpeg") or not shutil.which("ffprobe"):
        parser.error("FFmpeg and ffprobe must be installed and available on PATH")
    if not args.input.is_file():
        parser.error(f"input does not exist: {args.input}")
    if not 0 <= args.amount <= 100:
        parser.error("--amount must be between 0 and 100")

    info = probe(args.input)
    streams = info.get("streams", [])
    has_audio = any(s.get("codec_type") == "audio" for s in streams)
    has_video = any(s.get("codec_type") == "video" for s in streams)
    if not has_audio:
        parser.error("input contains no audio stream")

    output = args.output
    if output is None:
        output = args.input.with_name(f"{args.input.stem}_sweet{args.input.suffix}")
    if output.resolve() == args.input.resolve():
        parser.error("output must be different from input")
    output.parent.mkdir(parents=True, exist_ok=True)

    command = ["ffmpeg", "-hide_banner", "-y" if args.overwrite else "-n", "-i", str(args.input)]
    if has_video:
        command += ["-map", "0:v:0", "-map", "0:a:0", "-c:v", "copy"]
    else:
        command += ["-map", "0:a:0"]
    command += ["-af", sweet_filter(args.amount, args.pitch)]
    command += output_audio_codec(output)
    command += ["-map_metadata", "0"]
    if has_video:
        command += ["-shortest"]
    if output.suffix.lower() in {".mp4", ".m4v", ".mov"}:
        command += ["-movflags", "+faststart"]
    command += [str(output)]

    if args.dry_run:
        import shlex
        print(shlex.join(command))
        return 0

    print(f"Creating: {output}")
    completed = subprocess.run(command)
    if completed.returncode:
        return completed.returncode
    print(f"Done: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
