# Benchmarks

Each entry is one run of `cargo bench -p banshee --bench speech_out`, newest first. An entry
also holds the profile hotspots and the experiments made against that run.
[CONTRIBUTING.md](../CONTRIBUTING.md) states how to run it.

The bench speaks at the default `tts.speed`, which moved from 1.2 to 1.0 after 0.16.2. Slower
speech makes longer audio, so compare `kokoro` rows only between entries whose facts line names
the same speed.

## 2026-09-28

- Machine: Apple M5 Pro, macOS 26.6.2.
- Profile: `release`.
- Load averages at the start were 1.93, 6.91 and 12.62. No other heavy process ran.
- Each `kokoro` bench has its own `measurement_time`: 2, 7, 5 and 87 seconds.
- The full run took 219 seconds.
- Facts line: `kokoro: voice af_sky, speed 1.2, threads 8, espeak-ng found`

| Bench | Time (typical) | Throughput |
|---|---|---|
| text_path/short | 3.8198 µs | 10.236 MiB/s |
| text_path/medium | 5.7309 µs | 27.957 MiB/s |
| text_path/long | 24.985 µs | 36.567 MiB/s |
| g2p/short | 77.358 µs | 517.58 KiB/s |
| g2p/medium | 274.38 µs | 597.93 KiB/s |
| g2p/long | 761.24 µs | 1.2002 MiB/s |
| kokoro/warm_cache_load | 181.28 ms | n/a |
| kokoro/first_speech | 491.81 ms | n/a |
| kokoro/synthesize_short | 312.45 ms | 199.71 Kelem/s |
| kokoro/synthesize_long_windowed | 5.3762 s | 205.57 Kelem/s |

Realtime factor: synthesize_short 8.32x, synthesize_long_windowed 8.57x.

This run is faster than the 2026-09-27 run by about 40% for Kokoro. The load was lower. No
Kokoro code changed between the two runs.

### Profile

samply recorded `kokoro/synthesize_long_windowed` for 10 seconds on 2026-09-27, with the
`profiling` profile and the bench of the 2026-09-27 run. The table shows self time on the main thread.

| Self | Function | Role |
|---|---|---|
| 24% | `.LAdd.Compute16.x4.BlockBy4Loop`, under `MlasSgemmOperation` and `MlasConv` | matrix multiply for the convolutions |
| 17% | `Eigen::internal::psincos_float` | sine and cosine for the waveform |
| 9.7% | `onnxruntime::concurrency::SpinPause` | threads that spin while they wait |
| 5.5% | `onnxruntime::math::Im2col` | data layout for the convolutions |
| 5.2% | `ThreadPool::EndParallelSectionInternal` | threads that wait at the end of a parallel section |
| 4.4% | `onnxruntime::fft_radix2` | FFT for the waveform |
| 2.4% | `onnxruntime::bit_reverse` | FFT for the waveform |

- Almost all synthesis time is inside ONNX Runtime. The text path and G2P do not appear.
- About 15% of main-thread time is waiting in the thread pool, not compute.

### Experiment: ONNX Runtime spin-waiting off

The test set `session.intra_op.allow_spinning` to 0 against the default, on 2026-09-28. The runs alternated
on, off, on, off, and each ran `kokoro/synthesize` only.

| Run | Load (1 min) | synthesize_short | synthesize_long_windowed | User CPU |
|---|---|---|---|---|
| on | 3.61 | 311.36 ms | 5.2782 s | 721.95 s |
| off | 9.75 | 328.40 ms | 5.5314 s | 746.76 s |
| on | 19.92 | 327.12 ms | 5.3237 s | 736.01 s |
| off | 7.46 | 321.74 ms | 5.2606 s | 723.79 s |

- Result: no difference in time or CPU beyond the difference between rounds.
- The setting stays at the default.
- No second profile confirms that the setting took effect.

### Experiment: Kokoro thread cap 10 against 8

The test set `KOKORO_THREAD_CAP` to 10 against 8, on 2026-09-28. This machine has 15 cores: 5
performance and 10 efficiency. The runs alternated 8, 10, 8, 10, and each ran the `kokoro` group.

| Run | Load (1 min) | first_speech | synthesize_short | synthesize_long_windowed | User CPU |
|---|---|---|---|---|---|
| 8 | 6.88 | 695.00 ms | 470.88 ms | 7.3864 s | 1078.19 s |
| 10 | 5.93 | 700.73 ms | 417.17 ms | 7.3695 s | 1157.55 s |
| 8 | 8.60 | 657.78 ms | 437.74 ms | 7.4487 s | 1057.87 s |
| 10 | 6.80 | 607.23 ms | 446.32 ms | 7.8483 s | 1240.81 s |

- Result: no speed difference beyond the difference between rounds.
- 10 threads use 7% to 17% more user CPU than 8 threads.
- The cap stays at 8.
- The machine was not idle. Kokoro times are about 40% slower than the 2026-09-28 run above.

## 2026-09-27

- Machine: Apple M5 Pro, macOS 26.6.2.
- Profile: `release`.
- Load averages at the start were 7.34, 8.22 and 6.77. The machine was not idle.
- The `kokoro` group's `measurement_time` is 105 seconds.
- In this run, the `g2p` group runs G2P once over each whole text, not once per sentence.
- Facts line: `kokoro: voice af_sky, speed 1.2, threads 8, espeak-ng found`

| Bench | Time (typical) | Throughput |
|---|---|---|
| text_path/short | 5.7898 µs | 6.7534 MiB/s |
| text_path/medium | 8.8427 µs | 18.119 MiB/s |
| text_path/long | 40.435 µs | 22.595 MiB/s |
| g2p/short | 120.71 µs | 331.69 KiB/s |
| g2p/medium | 265.92 µs | 616.96 KiB/s |
| g2p/long | 1.1860 ms | 788.84 KiB/s |
| kokoro/warm_cache_load | 278.73 ms | n/a |
| kokoro/first_speech | 738.49 ms | n/a |
| kokoro/synthesize_short | 501.47 ms | 124.43 Kelem/s |
| kokoro/synthesize_long_windowed | 8.7600 s | 126.16 Kelem/s |

Criterion's typical estimate is the slope for the `text_path` and `g2p` groups. It is the mean
for the flat `kokoro` group.

Realtime factor: synthesize_short 5.18x, synthesize_long_windowed 5.26x.
