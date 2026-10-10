# Using a different model

Model-specific values live in `src/spec.rs`. To use another encoder-decoder, or one you trained yourself, put its ONNX files in a folder and add an optional `spec.json` (any omitted field keeps the pix2text-mfr default):

```json
{
  "name": "my-model",
  "encoder": "encoder_model.onnx",
  "decoder": "decoder_model.onnx",
  "tokenizer": "tokenizer.json",
  "input_width": 384, "input_height": 384,
  "mean": [0.5, 0.5, 0.5], "std": [0.5, 0.5, 0.5],
  "start_token": 2, "eos_token": 2,
  "base_url": null
}
```

then run `texcopy --model-dir path/to/folder`. The ONNX graphs must follow the Hugging Face `optimum` export convention: encoder `pixel_values` -> `last_hidden_state`; decoder `input_ids` + `encoder_hidden_states` -> `logits`, without a KV cache. Set `base_url` to a place that serves `{base_url}/{file}` if texcopy should download missing files, or `null` for local-only models.
