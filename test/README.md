# Feature experiments

One-off Python prototypes probing candidate Concat features before any of
them earns a Rust implementation. Each measures the thing that decides
shippability - speed against realtime on ordinary hardware, and output
quality by ear/eye - because per decision 0010 nothing ships that makes the
user install tools or wait on a server.

These are research scripts, not product code: run them from this directory,
point them at anything in `assets/` (gitignored), and read the numbers.

| Experiment | Probes | Candidate feature |
| --- | --- | --- |
| `silero_vad/test_vad.py` | Silero VAD speech regions + speed | silence removal, caption pre-pass |
| `rnnoise/test_denoise.py` | RNNoise denoise quality + speed | one-click "reduce noise" on a clip |
| `enhance_voice.py` | full repair chain: de-plosive, de-click (LPC interpolation), DeepFilterNet3/MossFormer2, ffmpeg polish | "Enhance voice" button |
| `sweet_voice.py` | the CapCut-style "Sweet" chain as pure ffmpeg filters | shipped as the `sweet` entry in the filters catalogue |
| `scene_detect/test_scene_detect.py` | PySceneDetect cut detection + speed | auto-split at scene cuts |
| `black_frames/detect_black_frames.py` | luminance-threshold black segment detection | trim dead frames, sanity-check exports |
| `face_blur/face_blur.py` | YuNet detection + Gaussian blur per frame | face blur effect |
| `hardware_analysis/analyze_hardware.py` | CPU/GPU/codec/hwaccel capability probe + tier score | first-run quality tier defaults (Rust port would use CPUID/wgpu, not subprocess probes) |

## Dependencies

Python 3.11+. Everything is per-experiment - install only what you're
poking at:

```sh
pip install silero-vad            # silero_vad (pulls onnxruntime, torch-free)
pip install pyrnnoise             # rnnoise
pip install numpy scipy           # enhance_voice (DSP stages)
pip install deepfilternet torch   # enhance_voice --backend deepfilternet
pip install clearvoice            # enhance_voice --backend clearvoice
pip install scenedetect[opencv]   # scene_detect
pip install opencv-python numpy   # black_frames, face_blur
pip install psutil                # hardware_analysis
```

`sweet_voice.py` and the polish stage of `enhance_voice.py` only need
`ffmpeg`/`ffprobe` on PATH. `face_blur` additionally needs the YuNet model
file (`face_detection_yunet_*.onnx` from the OpenCV zoo) beside it or via
`--model`.

## Findings worth keeping

- The `sweet` chain graduated: `desktop/src/lib/filters.ts` and
  `wolfcut-export::chains` carry it verbatim.
- `enhance_voice.py`'s stage *order* is the finding: surgical repairs
  (clicks, plosives) must run before the neural pass - networks trained on
  additive noise smear impulses instead of removing them - and loudness
  after it, because network output has arbitrary gain.
- Every detector here comfortably beats realtime on CPU except the neural
  denoisers, which are the reason a "processing..." progress bar exists.
