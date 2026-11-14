# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Compare permissive tokenizers for the from-scratch XIAR NeuralBrain.
Downloads ONLY tokenizer files (tokenizer.json / .tiktoken), never model weights.
Usage: python tokenizer_bench.py [--d-model 384] [--out results.json]"""
import argparse, json, pathlib, time
from huggingface_hub import hf_hub_download
from tokenizers import Tokenizer

CANDIDATES = [
    # name, source, id, license (of tokenizer files / repo)
    ("gpt2", "hf", "openai-community/gpt2", "MIT"),
    ("gpt-neox-20b", "hf", "EleutherAI/gpt-neox-20b", "Apache-2.0"),
    ("smollm2", "hf", "HuggingFaceTB/SmolLM2-135M", "Apache-2.0"),
    ("olmo2", "hf", "allenai/OLMo-2-0425-1B", "Apache-2.0"),
    ("mistral-v0.1", "hf", "mistralai/Mistral-7B-v0.1", "Apache-2.0"),
    ("mistral-tekken", "hf", "mistralai/Mistral-Nemo-Base-2407", "Apache-2.0"),
    ("qwen2.5", "hf", "Qwen/Qwen2.5-0.5B", "Apache-2.0"),
    ("qwen3", "hf", "Qwen/Qwen3-0.6B", "Apache-2.0"),
    ("cl100k", "tiktoken", "cl100k_base", "MIT"),
    ("o200k", "tiktoken", "o200k_base", "MIT"),
    # Not permissive (custom licenses) - measured for reference only if reachable:
    ("llama3", "hf", "meta-llama/Llama-3.2-1B", "Llama 3.2 Community (not permissive)"),
    ("gemma3", "hf", "google/gemma-3-1b-pt", "Gemma Terms (not permissive)"),
]

def load(src, rid):
    if src == "tiktoken":
        import tiktoken
        enc = tiktoken.get_encoding(rid)
        return (lambda s: enc.encode(s, disallowed_special=())), enc.n_vocab, (lambda ids: enc.decode(ids))
    p = hf_hub_download(rid, "tokenizer.json")
    tok = Tokenizer.from_file(p)
    return (lambda s: tok.encode(s, add_special_tokens=False).ids), tok.get_vocab_size(), (lambda ids: tok.decode(ids))

def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--d-model", type=int, default=384)
    ap.add_argument("--out", default="tokenizer_results.json"); a = ap.parse_args()
    samples = {p.stem: p.read_text(encoding="utf-8") for p in sorted(pathlib.Path(__file__).with_name("samples").glob("*.txt"))}
    rows = []
    for name, src, rid, lic in CANDIDATES:
        try:
            enc, vocab, dec = load(src, rid)
        except Exception as e:
            rows.append({"name": name, "id": rid, "license": lic, "error": type(e).__name__ + ": " + str(e)[:120]}); continue
        r = {"name": name, "id": rid, "license": lic, "vocab": vocab, "embed_params_M": round(vocab * a.d_model / 1e6, 2)}
        tot_t = tot_c = 0
        for k, s in samples.items():
            ids = enc(s); tot_t += len(ids); tot_c += len(s)
            r[f"tpc_{k}"] = round(len(ids) / len(s), 4)
            if k == "numbers":
                r["roundtrip_numbers"] = dec(ids) == s
        r["tpc_all"] = round(tot_t / tot_c, 4)
        r["digits_1234567"] = len(enc("1234567"))
        t0 = time.perf_counter(); [enc(samples["lessons"]) for _ in range(200)]
        r["enc_us_per_lesson_file"] = round((time.perf_counter() - t0) / 200 * 1e6, 1)
        rows.append(r)
    pathlib.Path(a.out).write_text(json.dumps({"d_model": a.d_model, "chars": {k: len(v) for k, v in samples.items()}, "rows": rows}, indent=2))
    for r in rows: print(json.dumps(r))
main()
