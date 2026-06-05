#!/usr/bin/env python3
"""Phase 3 confidence-gated escalation -- empirical threshold calibration.

Hits the resident grunt llama-server (granite-4.1-8b @ :8082) exactly the way
the production grunt path does -- POST /v1/chat/completions, logprobs:true,
top_logprobs:1, temperature OMITTED (so the server's launch-default temp is
used, matching tool_llm_prompt's `temperature: None`). Confidence is reduced
with the SAME formula as rift-bus `compute_confidence`: mean of per-token
logprobs -> exp() -> mean per-token probability in 0..1.

The dataset is bounded/checkable grunt work (classify / extract / arithmetic /
factual / code), deliberately mixing easy items (expect confident-correct) with
hard or trap items (letter-counting, Canberra, multi-digit multiply) that an 8B
gets wrong -- calibration needs WRONG cases across the confidence range or there
is nothing to separate.

Output: per-item table, correct-vs-wrong confidence distributions, a threshold
sweep (errors caught vs correct answers needlessly escalated), and a recommended
`confidence_threshold`. The number is a STARTING POINT for the human decision in
docs/PHASE3-confidence-gated-escalation.md -- not an auto-applied value.

Usage:  python docs/phase3-calibration.py [--endpoint http://127.0.0.1:8082] [--runs 1]
"""
import argparse
import json
import math
import re
import sys
import urllib.request

# ---------------------------------------------------------------------------
# Labeled dataset. type: "labeled" = accuracy-scored; "probe" = genuinely
# ambiguous (no single right answer) -- confidence observed but NOT scored.
# check: how to judge the normalized model output against `accept`.
#   numeric  -> extract first number from output, compare to accept numbers
#   word     -> normalized first token / whole, membership in accept set
#   contains -> any accept string appears in normalized output
# ---------------------------------------------------------------------------
ITEMS = [
    # --- easy classify (expect confident + correct) ---
    {"id": "sent-pos-1", "type": "labeled", "check": "word", "accept": ["positive"], "max": 6,
     "prompt": "Classify the sentiment as exactly one word, positive or negative: 'I absolutely love this app, it changed my life.'"},
    {"id": "sent-neg-1", "type": "labeled", "check": "word", "accept": ["negative"], "max": 6,
     "prompt": "Classify the sentiment as exactly one word, positive or negative: 'This is the worst purchase I have ever made.'"},
    {"id": "sent-pos-2", "type": "labeled", "check": "word", "accept": ["positive"], "max": 6,
     "prompt": "Classify the sentiment as exactly one word, positive or negative: 'Fast, reliable, and the support team was wonderful.'"},
    {"id": "sent-neg-2", "type": "labeled", "check": "word", "accept": ["negative"], "max": 6,
     "prompt": "Classify the sentiment as exactly one word, positive or negative: 'It crashed twice and lost all my data.'"},
    {"id": "cat-bug", "type": "labeled", "check": "word", "accept": ["bug"], "max": 6,
     "prompt": "Classify as exactly one word - bug, feature, or question: 'the app crashes when I tap save'"},
    {"id": "cat-feat", "type": "labeled", "check": "word", "accept": ["feature"], "max": 6,
     "prompt": "Classify as exactly one word - bug, feature, or question: 'it would be great if you added dark mode'"},
    {"id": "cat-q", "type": "labeled", "check": "word", "accept": ["question"], "max": 6,
     "prompt": "Classify as exactly one word - bug, feature, or question: 'how do I export my notes to PDF?'"},

    # --- easy factual / yes-no (expect confident + correct) ---
    {"id": "fact-planet", "type": "labeled", "check": "word", "accept": ["mercury"], "max": 6,
     "prompt": "Which planet is closest to the sun? Reply with only the planet name."},
    {"id": "fact-ww2", "type": "labeled", "check": "numeric", "accept": ["1945"], "max": 8,
     "prompt": "What year did World War 2 end? Reply with only the year."},
    {"id": "yn-spell", "type": "labeled", "check": "word", "accept": ["no"], "max": 4,
     "prompt": "Is 'recieve' spelled correctly? Reply yes or no."},
    {"id": "verb-past", "type": "labeled", "check": "word", "accept": ["went"], "max": 4,
     "prompt": "What is the past tense of 'go'? Reply with only the word."},

    # --- easy arithmetic (expect correct) ---
    {"id": "math-div", "type": "labeled", "check": "numeric", "accept": ["12"], "max": 6,
     "prompt": "What is 144 / 12? Reply with only the number."},
    {"id": "math-min", "type": "labeled", "check": "numeric", "accept": ["150"], "max": 6,
     "prompt": "How many minutes are in 2.5 hours? Reply with only the number."},

    # --- easy extraction (expect correct) ---
    {"id": "ext-email", "type": "labeled", "check": "contains", "accept": ["john.doe@acme.io"], "max": 14,
     "prompt": "Extract the email address from: 'contact me at john.doe@acme.io anytime'. Reply with only the email."},
    {"id": "ext-year", "type": "labeled", "check": "numeric", "accept": ["2019"], "max": 8,
     "prompt": "Extract the year from 'the product was released in March 2019 to wide acclaim'. Reply with only the year."},

    # --- easy code (expect correct) ---
    {"id": "code-len", "type": "labeled", "check": "numeric", "accept": ["5"], "max": 6,
     "prompt": "In Python, what does len('hello') return? Reply with only the number."},
    {"id": "code-map", "type": "labeled", "check": "contains", "accept": ["[2,4,6]", "2,4,6", "2, 4, 6"], "max": 14,
     "prompt": "In JavaScript, what does [1,2,3].map(x => x*2) return? Reply with only the resulting array."},

    # --- HARD / TRAP bounded items (likely WRONG -- populate the error cases) ---
    {"id": "trap-canberra", "type": "labeled", "check": "word", "accept": ["canberra"], "max": 6,
     "prompt": "What is the capital of Australia? Reply with only the city name."},
    {"id": "trap-strawberry-r", "type": "labeled", "check": "numeric", "accept": ["3"], "max": 6,
     "prompt": "How many times does the letter 'r' appear in the word 'strawberry'? Reply with only the number."},
    {"id": "trap-strawberry-len", "type": "labeled", "check": "numeric", "accept": ["10"], "max": 6,
     "prompt": "How many letters are in the word 'strawberry'? Reply with only the number."},
    {"id": "math-17x23", "type": "labeled", "check": "numeric", "accept": ["391"], "max": 8,
     "prompt": "What is 17 * 23? Reply with only the number."},
    {"id": "math-13x17", "type": "labeled", "check": "numeric", "accept": ["221"], "max": 8,
     "prompt": "What is 13 * 17? Reply with only the number."},
    {"id": "math-256x789", "type": "labeled", "check": "numeric", "accept": ["201984"], "max": 10,
     "prompt": "What is 256 * 789? Reply with only the number."},
    {"id": "trap-mississippi-s", "type": "labeled", "check": "numeric", "accept": ["4"], "max": 6,
     "prompt": "How many times does the letter 's' appear in 'mississippi'? Reply with only the number."},
    {"id": "ext-amount", "type": "labeled", "check": "contains", "accept": ["4,250", "4250"], "max": 12,
     "prompt": "Extract the dollar amount from 'the total came to $4,250.00 after tax'. Reply with only the amount."},
    {"id": "fact-elements", "type": "labeled", "check": "word", "accept": ["au"], "max": 6,
     "prompt": "What is the chemical symbol for gold? Reply with only the symbol."},

    # --- PROBE: genuinely ambiguous (observed, NOT accuracy-scored) ---
    {"id": "probe-meh", "type": "probe", "check": "word", "accept": [], "max": 6,
     "prompt": "Classify the sentiment as exactly one word, positive or negative: 'It's okay I guess, does the job.'"},
    {"id": "probe-flat", "type": "probe", "check": "word", "accept": [], "max": 6,
     "prompt": "Classify the sentiment as exactly one word, positive or negative: 'Well, that happened.'"},
    {"id": "probe-hotdog", "type": "probe", "check": "word", "accept": [], "max": 4,
     "prompt": "Is a hotdog a sandwich? Reply yes or no."},
    {"id": "probe-specbug", "type": "probe", "check": "word", "accept": [], "max": 6,
     "prompt": "Classify as exactly one word - bug or feature: 'the app does exactly what the spec says but every user hates it'"},
]


def normalize(s):
    return re.sub(r"[^a-z0-9@.,]+", " ", s.lower()).strip()


def first_number(s):
    m = re.findall(r"-?\d[\d,]*", s)
    return [x.replace(",", "") for x in m]


def judge(item, raw):
    out = normalize(raw)
    if item["check"] == "numeric":
        nums = first_number(raw)
        return any(a.replace(",", "") in nums for a in item["accept"])
    if item["check"] == "contains":
        return any(normalize(a) in out or a.replace(" ", "") in raw.replace(" ", "") for a in item["accept"])
    # word: accept if any accept token appears as a whole word. Strip punctuation
    # from tokens so "No," matches "no" (the comma-attached-token artifact).
    toks = re.findall(r"[a-z0-9]+", out)
    return any(a in toks or a == out for a in item["accept"])


def call(endpoint, prompt, max_tokens):
    body = json.dumps({
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
        "logprobs": True,
        "top_logprobs": 1,
        # temperature intentionally omitted -- matches the grunt path
        # (tool_llm_prompt sends temperature: None -> field skipped).
    }).encode()
    req = urllib.request.Request(endpoint + "/v1/chat/completions", data=body,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        d = json.loads(r.read())
    choice = d["choices"][0]
    content = choice["message"]["content"]
    lp = (choice.get("logprobs") or {}).get("content", [])
    if not lp:
        return content, None, None, None
    mean_lp = sum(t["logprob"] for t in lp) / len(lp)
    conf = min(1.0, max(0.0, math.exp(mean_lp)))          # mean per-token prob
    min_tp = min(math.exp(t["logprob"]) for t in lp)      # min per-token prob
    return content, conf, mean_lp, min_tp


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--endpoint", default="http://127.0.0.1:8082")
    ap.add_argument("--runs", type=int, default=1,
                    help="passes per item; >1 averages confidence (sampling noise)")
    args = ap.parse_args()

    rows = []
    for item in ITEMS:
        confs, lps, mtps, raws, corrects = [], [], [], [], []
        for _ in range(args.runs):
            try:
                content, conf, mlp, mtp = call(args.endpoint, item["prompt"], item["max"])
            except Exception as e:
                print(f"  ! {item['id']}: request failed: {e}", file=sys.stderr)
                content, conf, mlp, mtp = "<error>", None, None, None
            raws.append(content.strip().replace("\n", " "))
            if conf is not None:
                confs.append(conf); lps.append(mlp); mtps.append(mtp)
            corrects.append(judge(item, content) if item["type"] == "labeled" else None)
        conf = sum(confs) / len(confs) if confs else None
        mlp = sum(lps) / len(lps) if lps else None
        mtp = sum(mtps) / len(mtps) if mtps else None
        # majority correctness across runs
        correct = None
        if item["type"] == "labeled":
            correct = sum(1 for c in corrects if c) > len(corrects) / 2
        rows.append({"id": item["id"], "type": item["type"], "conf": conf,
                     "mlp": mlp, "mtp": mtp, "correct": correct, "out": raws[-1]})

    # ---- per-item table ----
    print("\n=== PER-ITEM ===")
    print(f"{'id':<22} {'type':<8} {'mean':>6} {'minTP':>6} {'mlp':>8}  {'ok':<5} output")
    for r in rows:
        c = f"{r['conf']:.4f}" if r["conf"] is not None else "  n/a"
        mt = f"{r['mtp']:.4f}" if r["mtp"] is not None else "  n/a"
        m = f"{r['mlp']:.3f}" if r["mlp"] is not None else "   n/a"
        ok = "" if r["correct"] is None else ("OK" if r["correct"] else "WRONG")
        print(f"{r['id']:<22} {r['type']:<8} {c:>6} {mt:>6} {m:>8}  {ok:<5} {r['out'][:44]}")

    labeled = [r for r in rows if r["type"] == "labeled" and r["conf"] is not None]
    correct = [r for r in labeled if r["correct"]]
    wrong = [r for r in labeled if not r["correct"]]
    probes = [r for r in rows if r["type"] == "probe" and r["conf"] is not None]

    def stats(name, xs):
        if not xs:
            print(f"  {name}: (none)")
            return
        cs = sorted(x["conf"] for x in xs)
        mean = sum(cs) / len(cs)
        print(f"  {name}: n={len(xs)} min={cs[0]:.4f} mean={mean:.4f} max={cs[-1]:.4f}")

    print("\n=== DISTRIBUTIONS ===")
    print(f"  labeled accuracy: {len(correct)}/{len(labeled)} = {100*len(correct)/max(1,len(labeled)):.0f}%")
    stats("CORRECT  mean-conf", correct)
    stats("WRONG    mean-conf", wrong)
    stats("PROBE    mean-conf", probes)

    # ---- threshold sweep (both metrics) ----
    def sweep(key, label):
        print(f"\n=== THRESHOLD SWEEP on {label} (escalate when {key} < t) ===")
        print(f"{'t':>6} {'errors_caught':>14} {'correct_escalated':>18} {'precision':>10}")
        best = None
        for i in range(50, 100, 3):
            t = i / 100.0
            caught = sum(1 for r in wrong if r[key] is not None and r[key] < t)
            false_esc = sum(1 for r in correct if r[key] is not None and r[key] < t)
            total_esc = caught + false_esc
            prec = (caught / total_esc) if total_esc else None
            flag = ""
            if wrong and caught / len(wrong) >= 0.6 and (total_esc == 0 or (prec or 0) >= 0.5):
                if best is None:
                    best = t
                    flag = "  <-- candidate"
            pp = f"{prec:.2f}" if prec is not None else "  -"
            print(f"{t:>6.2f} {caught:>9}/{len(wrong):<4} {false_esc:>13}/{len(correct):<4} {pp:>10}{flag}")
        return best

    best = sweep("conf", "MEAN per-token probability (the shipped metric)")
    best_min = sweep("mtp", "MIN per-token probability (spec open-decision #1)")

    print("\n=== RECOMMENDATION ===")
    if not wrong:
        print("  No wrong answers in this run -- granite aced the bounded set.")
        print("  Either the dataset is too easy (add harder traps) or grunt work")
        print("  genuinely doesn't need confidence-gating. Re-run with --runs 3 or")
        print("  harder items before setting a threshold.")
    elif best is not None or best_min is not None:
        if best is not None:
            print(f"  MEAN-prob candidate threshold ~= {best:.2f}")
        if best_min is not None:
            print(f"  MIN-prob  candidate threshold ~= {best_min:.2f}")
        print("  A candidate emerged -- validate it against the sweep precision column")
        print("  and the cost gate (escalation target is partner/cloud) before enabling.")
        print("  Set confidence_threshold in EnsembleConfig only if precision holds on a")
        print("  larger sample (this run is small -- re-run with more/harder items).")
    else:
        print("  NEITHER mean nor min per-token probability separates correct from wrong.")
        print("  Logprob confidence is miscalibrated on this set -- the model is")
        print("  confidently WRONG (the spec's load-bearing caveat, now empirical).")
        print("  RECOMMENDATION: keep confidence_threshold: None (do NOT enable).")
        print("  Gating would escalate confident-correct answers to cloud while missing")
        print("  the confident-wrong ones -- net cost, no accuracy gain on grunt work.")
    print()


if __name__ == "__main__":
    main()
