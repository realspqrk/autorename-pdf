# Example configs

Small 4.x recipes: each file lists only what differs from the defaults. The full, commented reference is [`config.yaml.example`](../config.yaml.example).

Copy one to your config location (`autorename-pdf config path` prints it), then set your API key (or `${VAR}` in a `.env` beside the config) and `company.names`.

| File | Use when | Example filename |
|------|----------|------------------|
| [`academic.yaml`](academic.yaml) | Papers, preprints, theses | `20260101 Smith Nature Attention Is All You Need.pdf` |
| [`er-ar.yaml`](er-ar.yaml) | German ER/AR (Eingangsrechnung / Ausgangsrechnung) | `20260906 ACME ER EUR 12,13.pdf` |
| [`invoice-id.yaml`](invoice-id.yaml) | Keep the total and add the printed invoice number | `20260906 ACME AP EUR 12.13 12345.pdf` |
| [`ollama-offline.yaml`](ollama-offline.yaml) | Fully local: Ollama + built-in OCR, no cloud requests | `20260906 ACME AP EUR 12.13.pdf` |
| [`vision-cloud.yaml`](vision-cloud.yaml) | Scans read by a cloud vision model, no local OCR | `20260906 ACME AP EUR 12.13.pdf` |

One run with another profile, without copying a file:

```bash
autorename-pdf rename --profile academic paper.pdf
```

Validate a recipe before using it:

```bash
autorename-pdf --config examples/academic.yaml config validate
```

`cargo test -p autorename-app --test examples` validates every file here in CI.
