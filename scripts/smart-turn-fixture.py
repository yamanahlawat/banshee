# Writes the Whisper log-mel features of the VAD test clip, as transformers computes them
# for Smart Turn (chunk_length=8), and prints P(complete) when given the model path.
import sys
import numpy as np, onnxruntime as ort, soundfile as sf, transformers
from transformers import WhisperFeatureExtractor

print("transformers", transformers.__version__)
print("numpy", np.__version__)
print("onnxruntime", ort.__version__)

ROOT = __file__.rsplit("/scripts/", 1)[0]
N = 8 * 16000

pcm = sf.read(ROOT + "/bansheed/tests/data/vad_speech_16k.wav", dtype="int16")[0]
audio = (pcm.astype(np.float32) / 32768.0)
audio = audio[-N:] if len(audio) > N else np.pad(audio, (N - len(audio), 0))

fe = WhisperFeatureExtractor(chunk_length=8)
out = fe(audio, sampling_rate=16000, return_tensors="np", padding="max_length", max_length=N, truncation=True, do_normalize=True)
feats = out.input_features.astype(np.float32)
assert feats.shape == (1, 80, 800), feats.shape
feats[0].astype("<f4").tofile(ROOT + "/bansheed/tests/data/smart_turn_features.f32")

if len(sys.argv) > 1:
    sess = ort.InferenceSession(sys.argv[1], providers=["CPUExecutionProvider"])
    p = sess.run(None, {"input_features": feats})[0].reshape(-1)[0].item()
    print("P(complete)", repr(p))
