#!/usr/bin/env node
// coding-eval.mjs — objective coding-quality benchmark for local coder models.
//
// Sends a fixed set of Python tasks to a model's llama-server, extracts the
// generated function, EXECUTES it against hidden unit tests, and scores
// pass/fail + latency. Correctness is objective (the code runs or it doesn't).
//
// USAGE (one model resident at a time on 16GB):
//   node tools/coding-eval.mjs <port> <label> [reasoning_effort]
//   e.g. node tools/coding-eval.mjs 8086 gpt-oss-20b medium   # gpt-oss w/ medium effort
//        node tools/coding-eval.mjs 8087 qwen-coder-14b
//
// reasoning_effort (optional 3rd arg): sets chat_template_kwargs.reasoning_effort
// (for gpt-oss). When omitted, thinking is disabled (enable_thinking:false) for
// the non-reasoning coders.

import { writeFileSync, mkdtempSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const port = process.argv[2];
const label = process.argv[3] || `:${port}`;
const effort = process.argv[4]; // e.g. "medium" for gpt-oss
if (!port) { console.error("usage: node tools/coding-eval.mjs <port> <label> [reasoning_effort]"); process.exit(2); }
const BASE = `http://127.0.0.1:${port}`;

const EASY_TASKS = [
  {
    name: "two_sum (easy)",
    prompt: "Write a Python function `two_sum(nums, target)` that returns the indices [i, j] (i<j) of the two numbers in `nums` that add up to `target`. Exactly one solution exists. Output ONLY the function inside a ```python code block.",
    test: `
assert sorted(two_sum([2,7,11,15],9))==[0,1]
assert sorted(two_sum([3,2,4],6))==[1,2]
assert sorted(two_sum([3,3],6))==[0,1]
`,
  },
  {
    name: "merge_intervals (medium)",
    prompt: "Write a Python function `merge_intervals(intervals)` that merges overlapping intervals (each a [start,end] list) and returns the merged list sorted by start. Touching intervals like [1,2],[2,3] merge into [1,3]. Output ONLY the function inside a ```python code block.",
    test: `
assert merge_intervals([[1,3],[2,6],[8,10],[15,18]])==[[1,6],[8,10],[15,18]]
assert merge_intervals([[1,4],[4,5]])==[[1,5]]
assert merge_intervals([])==[]
assert merge_intervals([[1,4],[0,4]])==[[0,4]]
`,
  },
  {
    name: "is_valid parens (medium)",
    prompt: "Write a Python function `is_valid(s)` that returns True if the string `s` of brackets among ()[]{} is correctly matched and nested, else False. Output ONLY the function inside a ```python code block.",
    test: `
assert is_valid("()[]{}")==True
assert is_valid("(]")==False
assert is_valid("([)]")==False
assert is_valid("{[]}")==True
assert is_valid("")==True
assert is_valid("(")==False
`,
  },
  {
    name: "edit_distance (hard/DP)",
    prompt: "Write a Python function `edit_distance(a, b)` returning the Levenshtein edit distance (minimum single-character insertions, deletions, or substitutions) between strings `a` and `b`. Output ONLY the function inside a ```python code block.",
    test: `
assert edit_distance("horse","ros")==3
assert edit_distance("intention","execution")==5
assert edit_distance("","abc")==3
assert edit_distance("abc","abc")==0
`,
  },
];

const HARD_TASKS = [
  {
    name: "eval_expr (parser+unary)",
    prompt: "Write a Python function `eval_expr(s)` that evaluates an arithmetic expression string with integers, binary + - * /, parentheses, standard precedence (* / before + -, left-associative), AND unary minus (e.g. -3, 3*-2). Use true division for /. Do NOT use eval/exec. Output ONLY the function in a ```python block.",
    test: `
assert eval_expr("2+3*4")==14
assert eval_expr("(2+3)*4")==20
assert eval_expr("2*(3+4*2)")==22
assert eval_expr("1+2-3*4")==-9
assert eval_expr("-3+5")==2
assert eval_expr("3*-2")==-6
assert eval_expr("100/(2+3)/2")==10
`,
  },
  {
    name: "parse_csv (RFC quotes)",
    prompt: 'Write a Python function `parse_csv(line)` that splits one CSV line into a list of fields, honoring double-quoted fields (which may contain commas) and escaped quotes (a doubled "" inside a quoted field is a literal quote). Output ONLY the function in a ```python block.',
    test: `
assert parse_csv('a,b,c')==['a','b','c']
assert parse_csv('a,"b,c",d')==['a','b,c','d']
assert parse_csv('"d""e"')==['d"e']
assert parse_csv('a,,b')==['a','','b']
assert parse_csv('"hello, world",42')==['hello, world','42']
`,
  },
  {
    name: "simplify_path (unix)",
    prompt: 'Write a Python function `simplify_path(path)` that canonicalizes a Unix absolute path: collapse multiple slashes, resolve "." (current) and ".." (parent, not above root), no trailing slash (except root "/"). Note "..." is an ordinary name. Output ONLY the function in a ```python block.',
    test: `
assert simplify_path("/home/")=="/home"
assert simplify_path("/../")=="/"
assert simplify_path("/home//foo/")=="/home/foo"
assert simplify_path("/a/./b/../../c/")=="/c"
assert simplify_path("/...")=="/..."
`,
  },
  {
    name: "min_window (hard)",
    prompt: "Write a Python function `min_window(s, t)` returning the smallest substring of `s` that contains every character of `t` including multiplicity; return '' if none. Output ONLY the function in a ```python block.",
    test: `
assert min_window("ADOBECODEBANC","ABC")=="BANC"
assert min_window("a","a")=="a"
assert min_window("a","aa")==""
assert min_window("aa","aa")=="aa"
`,
  },
  {
    name: "regex is_match (.* DP)",
    prompt: "Write a Python function `is_match(s, p)` implementing regex matching where '.' matches any single char and '*' matches zero or more of the PRECEDING element; the match must cover the ENTIRE string s. Output ONLY the function in a ```python block.",
    test: `
assert is_match("aa","a")==False
assert is_match("aa","a*")==True
assert is_match("ab",".*")==True
assert is_match("aab","c*a*b")==True
assert is_match("mississippi","mis*is*p*.")==False
assert is_match("","")==True
`,
  },
];

const TASKS = process.env.EVAL_HARD ? HARD_TASKS : EASY_TASKS;

function extractCode(content) {
  // First fenced python (or generic) block; fallback to whole content.
  const m = content.match(/```(?:python|py)?\s*\n([\s\S]*?)```/i);
  return (m ? m[1] : content).trim();
}

async function runTask(t, dir, i) {
  const ck = effort ? { reasoning_effort: effort } : { enable_thinking: false };
  const body = {
    model: "local",
    messages: [
      { role: "system", content: "You are an expert Python programmer. Return correct, working code." },
      { role: "user", content: t.prompt },
    ],
    temperature: 0.2,
    max_tokens: process.env.MAX_TOKENS ? parseInt(process.env.MAX_TOKENS) : 4096,
    chat_template_kwargs: ck,
  };
  const started = Date.now();
  let content = "";
  try {
    const r = await fetch(`${BASE}/v1/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: "Bearer none" },
      body: JSON.stringify(body),
    });
    if (!r.ok) return { ok: false, why: `HTTP ${r.status}`, ms: Date.now() - started };
    const j = await r.json();
    content = j?.choices?.[0]?.message?.content || "";
  } catch (e) {
    return { ok: false, why: `fetch: ${e.message}`, ms: Date.now() - started };
  }
  const ms = Date.now() - started;
  const code = extractCode(content);
  if (!code) return { ok: false, why: "no code", ms };
  const file = join(dir, `t${i}.py`);
  writeFileSync(file, code + "\n" + t.test + '\nprint("PASS")\n');
  try {
    const out = execFileSync("python", [file], { timeout: 15000, stdio: ["ignore", "pipe", "pipe"] }).toString();
    return { ok: out.includes("PASS"), why: out.includes("PASS") ? "tests pass" : "no PASS", ms, chars: code.length };
  } catch (e) {
    const err = (e.stderr?.toString() || e.message || "").trim().split("\n").pop();
    return { ok: false, why: `runtime: ${err.slice(0, 80)}`, ms, chars: code.length };
  }
}

(async () => {
  try {
    const h = await fetch(`${BASE}/health`).then((r) => r.status).catch(() => null);
    if (h == null) { console.error(`No server at ${BASE}`); process.exit(2); }
  } catch {}
  const dir = mkdtempSync(join(tmpdir(), "ceval-"));
  console.log(`\n=== coding eval — ${label} (${BASE})${effort ? ` effort=${effort}` : ""} ===`);
  let pass = 0, totMs = 0;
  for (let i = 0; i < TASKS.length; i++) {
    const r = await runTask(TASKS[i], dir, i);
    if (r.ok) pass++;
    totMs += r.ms;
    console.log(`[${r.ok ? "PASS" : "FAIL"}] ${TASKS[i].name.padEnd(24)} ${String(r.ms).padStart(7)}ms  ${r.chars ? r.chars + "ch  " : ""}${r.why}`);
  }
  console.log(`--- ${label}: ${pass}/${TASKS.length} solved | total ${(totMs / 1000).toFixed(1)}s ---\n`);
})();
