# Using a different model

The values that are specific to a model live in `src/spec.rs`. To use another encoder-decoder model, or one you trained yourself, put its ONNX files in a folder and add a `spec.json` next to them. Anything you leave out keeps the pix2text-mfr default.

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

Then run `texcopy --model-dir path/to/folder`.

The ONNX graphs have to follow the Hugging Face `optimum` export layout: the encoder takes `pixel_values` and gives `last_hidden_state`, and the decoder takes `input_ids` + `encoder_hidden_states` and gives `logits`, with no KV cache. Set `base_url` to somewhere that serves `{base_url}/{file}` if you want texcopy to download missing files, or `null` for a local-only model.
