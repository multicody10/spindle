# Model Configuration and Optimization

## Separate configuration domains

Spindle keeps load-time runtime configuration separate from generation parameters.

Runtime values currently modeled include context length, GPU layers, CPU and batch threads, evaluation and micro-batch sizes, parallel slots, Flash Attention, KV cache types and placement, memory mapping, and memory locking.

Generation values currently modeled include temperature, top-p, top-k, min-p, typical-p, repetition penalty, repeat-last-N, presence and frequency penalties, output token limit, seed, and stop sequences.

The existence of a field in the schema does not imply every provider supports it. Once provider adapters are introduced, the capability registry must determine which values can be applied and how each field maps to that provider's actual API or process arguments.

## Optimizer inputs

The foundation optimizer uses:

- GGUF architecture metadata when present: layer count, declared context, embedding dimensions, attention heads, KV heads, and key/value head dimensions.
- Model file size as a rough proxy for weight storage, not as an exact resident memory measurement.
- Total and currently available Windows system memory.
- NVIDIA total and currently free VRAM when nvidia-smi is available.
- The selected optimization profile.

The KV-cache estimate is derived from layer count, KV heads, key/value dimensions, context length, sequence slots, and an assumed two-byte cache element. This is an estimate, not a runtime measurement. The model loader must later compare estimates against llama.cpp's reported effective allocation and recalibrate.

## Optimization profiles

- Automatic: conservative context and headroom-aware placement.
- Maximum performance: larger batches and limited parallelism only when resource thresholds allow.
- Balanced: moderate context and batch sizes.
- Memory efficient: small context, batch, and concurrency.
- Maximum context: try the model's declared context limit, then reduce until the estimated memory budget is satisfied.
- Custom: preserve manual values.

These are different from sampling presets. Changing temperature does not optimize hardware usage; changing GPU offload does not change sampling behavior.

## Current limitations

- The only GPU telemetry backend is NVIDIA nvidia-smi.
- This version does not parse every GGUF tensor or quantization encoding, and it does not read model weights into memory.
- Memory estimates cannot account exactly for every architecture, tensor layout, allocator, backend, or driver.
- No benchmark or calibration request is run.
- No inference runtime is launched. Values are stored as recommendations and are not presented as applied settings.
