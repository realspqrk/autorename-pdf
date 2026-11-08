# Provider API Reference (structured output + vision) for the Rust reqwest client

Accessed: 2026-11-07. Sources are primary docs only (URLs at bottom). Items marked (UNVERIFIED) were not stated on the fetched page.
Doc pages were summarised by a fetch tool, so spot-check exact field names against the live docs before shipping.

## 0. Key findings

- CLAUDE.md says OpenAI default is `gpt-5.6-luna`. It is NOT on the current OpenAI models page. Current cheap tier is `gpt-6-luna` ($0.10 in / $0.50 out per MTok). Treat `gpt-5.6-luna` as stale and update the default.
- Anthropic: `claude-haiku-5-5` (released 2026-10-07) is the cheap/fast pick. It is dateless, which means a pinned snapshot.
- Gemini: `generateContent` is "legacy, still supported". The Interactions API (`/v1beta/interactions`) is now Google's default. The OpenAI-compat endpoint is "beta". Recommendation is in section 3.
- xAI default is `grok-4.7`. Docs list `grok-build-0.1`, `grok-4.3` and `grok-4.20-0309-non-reasoning` as cheaper text models, but do not state image/structured-output support for them.

## 1. Comparison table

| | OpenAI | Anthropic | Gemini (generateContent) | xAI | Ollama |
|---|---|---|---|---|---|
| URL | `POST https://api.openai.com/v1/responses` | `POST https://api.anthropic.com/v1/messages` | `POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent` | `POST https://api.x.ai/v1/responses` (or `/v1/chat/completions`, OpenAI-compatible) | `POST http://localhost:11434/api/chat` |
| Auth | `Authorization: Bearer $KEY` | `x-api-key: $KEY`, `anthropic-version: 2026-10-21`, `content-type: application/json` | `x-goog-api-key: $KEY` | `Authorization: Bearer $KEY` | none (local) |
| Default model (cheap+fast) | `gpt-6-luna` (alt: `gpt-6.1-terra` $1/$5, better vision) | `claude-haiku-5-5` ($0.10/$0.50; alt `claude-sonnet-5-5` $2/$10) | `gemini-3.5-flash-lite` (stable; alt `gemini-3.6-flash`, `gemini-3.8-flash`) | `grok-4.7` ($2/$6) | `gemma4` (named in Ollama vision docs) or `qwen3.8` (vision+tools+thinking, 27b); pull tags locally, (UNVERIFIED) which fits extraction best |
| Structured field | `text.format = {type:"json_schema", name, schema, strict:true}` | `output_config.format = {type:"json_schema", schema}` (GA, no beta header) | `generationConfig.responseFormat.text = {mimeType:"APPLICATION_JSON", schema}`; SDKs map to `response_mime_type` + `response_json_schema` | Responses: `text.format` same as OpenAI. Chat: `response_format = {type:"json_schema", json_schema:{name,schema,strict}}` | `format: <JSON Schema object>` (also accepts `"json"`) |
| Image | `{"type":"input_image","image_url":"data:image/png;base64,...","detail":"auto"}` in user `content` | `{"type":"image","source":{"type":"base64","media_type":"image/png","data":"..."}}` | `{"inlineData":{"mimeType":"image/png","data":"<b64>"}}` in `parts` | `{"type":"input_image","image_url":"data:image/png;base64,...","detail":"high"}` | `"images":["<b64 no prefix>"]` on the message |
| Image limits | PNG/JPEG/WEBP/GIF; 512 MB request; 1500 images | 5 MB/img on 3rd-party clouds (first-party limits not captured) | (UNVERIFIED) | JPG/PNG only; 20 MiB/img | model-dependent |
| Storage off | `"store": false` | n/a (no stored responses) | n/a | `"store": false` (stored 30 days by default); do not store when sending images or the request may fail | n/a |
| Reasoning off/low | `"reasoning":{"effort":"none"}` on gpt-6-luna. Allowed: none, low, medium, high, xhigh, max. Default medium | `"thinking":{"type":"disabled"}` accepted on Haiku 5.5 at effort high or below. Effort is `output_config.effort` (low, medium, high, xhigh, max; default medium). `"enabled"` is rejected (400) | `generationConfig.thinkingConfig.thinkingLevel`: MINIMAL, LOW, MEDIUM, HIGH. Cannot fully disable on Gemini 3 (compat docs) | `reasoning.effort` low, medium, high, xhigh (default high on grok-4.7) | `"think": false` |
| Temperature | Not stated for gpt-6-luna; omit it (UNVERIFIED) | Omit `temperature`, `top_p`, `top_k` on Haiku 5.5 | Ignored on Gemini 3.6 Flash and newer | Not stated; omit | `options.temperature` works; use 0 |
| Max tokens | `max_output_tokens` (includes reasoning; reserve generously) | `max_tokens` (required; thinking counts toward it) | `generationConfig.maxOutputTokens` | `max_output_tokens` | `options.num_predict` |
| Output location | `output[].content[].text` where type `output_text` (also `output_text` convenience in SDKs only) | `content[].text` (type `text`) | `candidates[0].content.parts[0].text` | same as OpenAI | `message.content` |
| Request-id header | `x-request-id` | `request-id` (also `request_id` in error body) | none documented | none documented | none |
| Retry hints | `Retry-After` (429/503); `x-ratelimit-{limit,remaining,reset}-{requests,tokens}` | `retry-after`; `anthropic-ratelimit-{requests,tokens,input-tokens,output-tokens}-{limit,remaining,reset}` | none documented | none documented | none |

## 2. Failure signals

| Provider | Refusal | Truncation / incomplete | Auth | Rate limit | Overload / 5xx |
|---|---|---|---|---|---|
| OpenAI | `output[].content[]` part `type:"refusal"` (not schema-valid; check before parse) | top-level `status:"incomplete"`, `incomplete_details.reason:"max_output_tokens"` (may have no visible text) | 401 | 429: retryable (`rate_limit_exceeded`, `slow_down`) vs NOT retryable (`error.code` in `credit_balance_exhausted`, `*_spend_limit_exceeded`, `insufficient_quota`) | 500, 503 `server_is_overloaded` (honor Retry-After) |
| Anthropic | HTTP 200, `stop_reason:"refusal"` (billed; output may not match schema) | `stop_reason:"max_tokens"` (invalid JSON possible; raise `max_tokens`) | 401 `authentication_error`; 403 `permission_error`; 402 `billing_error` | 429 `rate_limit_error`. Spend-cap 429 has `error.details.error_code:"enforced_spend_limit_reached"` and NO retry-after: do not retry. Own spend limit returns 400 | 500 `api_error`, 504 `timeout_error`, 529 `overloaded_error`; 413 `request_too_large` |
| Gemini | `promptFeedback.blockReason` set and no candidates; or `finishReason` SAFETY, PROHIBITED_CONTENT, SPII, BLOCKLIST, RECITATION | `finishReason:"MAX_TOKENS"` | 400 (bad key) / 403 `PERMISSION_DENIED` | 429 `RESOURCE_EXHAUSTED` (backoff with jitter) | 500, 503 `UNAVAILABLE`, 504 |
| xAI | (UNVERIFIED) assume OpenAI-style | (UNVERIFIED) assume `status:"incomplete"` | 401 | 429 | 5xx |
| Ollama | none | `done_reason` (example shows `stop`; values not listed, treat `length` as truncated, UNVERIFIED) | n/a | n/a | connection refused if not running |

Error bodies: Anthropic `{"type":"error","error":{"type","message"},"request_id"}`; Gemini `{"error":{"code","message"}}` (new api-errors page, snake_case code).
Retry policy for the client: retry 429 (only if not a quota/spend error), 5xx, 529, timeouts. Use `Retry-After` when present, else exponential backoff with jitter. Never retry 400/401/403/402.

## 3. Schema subset restrictions

- OpenAI strict: every object needs `additionalProperties:false` and ALL keys listed in `required`. Optional fields are modelled as `"type":["string","null"]`. `$ref` and enum are supported. (Full unsupported-keyword list was on a part of the page that was not read; keep the schema to type/properties/required/enum/array/items.)
- Anthropic: supported types object/array/string/integer/number/boolean/null, `enum`, `const`, `anyOf`, internal `$ref`/`$defs`, `additionalProperties` only as `false`, string `format` date, date-time, etc., array `minItems` 0 or 1. NOT supported: recursion, `minimum`/`maximum`/`multipleOf`, `minLength`/`maxLength`, external `$ref`. Violations return 400. Enum string casing can change in output, so compare case-insensitively. Do not ask for step-by-step reasoning in a property (`reasoning_extraction` refusal).
- Gemini: types incl. null via type array; object `properties/required/additionalProperties`; string `enum/format(date-time,date,time)`; number `minimum/maximum`; array `items/prefixItems/minItems/maxItems`. Unsupported keywords are silently ignored (validate in app). Deep or large schemas give 400 `INVALID_ARGUMENT`. Output key order follows schema key order.
- xAI: supports anyOf/oneOf/allOf(single), `$ref`/`$defs` (non-circular); `additionalProperties` defaults to false; minLength/maxLength up to 2048, minItems/maxItems up to 256. 400 on empty `enum`/`anyOf`, `items` as an array.
- Ollama: any JSON Schema; also put the schema text in the prompt (docs recommend this). Ollama cloud does not support structured outputs.

Common safe subset for ONE shared schema: object with string/null-union/enum/array props, all keys required, `additionalProperties:false`, no numeric/length constraints. Validate everything again locally.

## 4. Gemini endpoint recommendation

Use native `generateContent` (not the OpenAI-compat endpoint).
- Reasons: the compat layer (`https://generativelanguage.googleapis.com/v1beta/openai/`) is documented as beta, and its reasoning controls are restricted (cannot be turned off on Gemini 3). Native gives `thinkingConfig` and `finishReason`/`promptFeedback`.
- Caveat: Google calls `generateContent` "legacy, still supported" and the Interactions API (`POST /v1beta/interactions`, headers `x-goog-api-key` and `Api-Revision: 2026-05-20`, body `{model, input, response_format:{type:"text", mime_type:"application/json", schema}}`, images as `{type:"image", mime_type, data}`, output via `output_text`/`steps`) the default. No shutdown date for `generateContent` is given. Isolate Gemini behind a trait so the swap is cheap.
- Request field names differ between the docs' REST example (`generationConfig.responseFormat.text`) and the API reference (`responseMimeType` + `responseJsonSchema`). Test both against a live key and keep the one that works.

## 5. Minimal examples

OpenAI Responses request:
```json
{"model":"gpt-6-luna","store":false,"reasoning":{"effort":"none"},"max_output_tokens":2000,
 "input":[{"role":"user","content":[{"type":"input_text","text":"Extract fields"},
   {"type":"input_image","image_url":"data:image/png;base64,AAAA","detail":"auto"}]}],
 "text":{"format":{"type":"json_schema","name":"doc","strict":true,
   "schema":{"type":"object","properties":{"company":{"type":"string"},"date":{"type":["string","null"]}},
             "required":["company","date"],"additionalProperties":false}}}}
```
OpenAI response (shape): `{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"{...}"}]}]}`; refusal part is `{"type":"refusal","refusal":"..."}`.

Anthropic request (headers: `x-api-key`, `anthropic-version: 2026-10-21`):
```json
{"model":"claude-haiku-5-5","max_tokens":2000,"thinking":{"type":"disabled"},
 "output_config":{"effort":"low","format":{"type":"json_schema","schema":{"type":"object",
   "properties":{"company":{"type":"string"}},"required":["company"],"additionalProperties":false}}},
 "messages":[{"role":"user","content":[
   {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}},
   {"type":"text","text":"Extract fields"}]}]}
```
Response: `{"content":[{"type":"text","text":"{\"company\":\"...\"}"}],"stop_reason":"end_turn"}`.

Gemini generateContent request (condensed from docs; images go in `parts`):
```json
{"contents":[{"parts":[{"text":"Extract fields"},{"inlineData":{"mimeType":"image/png","data":"AAAA"}}]}],
 "generationConfig":{"responseFormat":{"text":{"mimeType":"APPLICATION_JSON","schema":{"type":"object",
   "properties":{"company":{"type":"string"}},"required":["company"]}}},"thinkingConfig":{"thinkingLevel":"MINIMAL"}}}
```
xAI Responses: same body as OpenAI minus `reasoning.effort:"none"` (not offered; use `"low"`), `model:"grok-4.7"`, `store:false` only when no images and no continuation needed.

Ollama:
```json
{"model":"gemma4","stream":false,"think":false,"options":{"temperature":0},
 "messages":[{"role":"user","content":"Extract fields","images":["AAAA"]}],
 "format":{"type":"object","properties":{"company":{"type":"string"}},"required":["company"]}}
```

## 6. Sources (all accessed 2026-11-07)

- https://developers.openai.com/api/docs/models and /models/gpt-6-luna
- https://developers.openai.com/api/docs/guides/structured-outputs, /guides/images-vision, /guides/reasoning, /guides/error-codes, /guides/rate-limits
- https://platform.claude.com/docs/en/build-with-claude/structured-outputs, /effort, /vision
- https://platform.claude.com/docs/en/about-claude/models/overview, /models/haiku-5-5/overview, /api/errors, /api/rate-limits
- https://aistudio.google.com/docs/models, /docs/structured-output(.md?generate-content=true), /docs/openai, /docs/api-errors.md, /docs/migrate-to-interactions.md, /api-reference/generate-content, /llms.txt (ai.google.dev 301-redirects here)
- https://docs.x.ai/docs/models, /models/grok-4.7, /guides/structured-outputs, /guides/image-understanding, /guides/chat, /key-information/debugging
- https://docs.ollama.com/capabilities/structured-outputs, /capabilities/vision, /api/chat; https://ollama.com/search?c=vision
