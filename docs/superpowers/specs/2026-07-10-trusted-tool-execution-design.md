# Trusted Tool Execution Design

## Problem

deepcli currently mixes model-supplied tool arguments with host authorization. Several provider schemas expose `approved`, `writes_files`, and `requires_network`; the executor trusts those values, and `run_tests` supplies approval internally for an arbitrary shell command. A model can therefore authorize its own writes or destructive commands. Approval records are not bound to a concrete invocation, path checks are lexical, ignored files can be read directly, and failed commands are reported as successful tool executions.

The native terminal also routes short prompts through a tool-less provider path. This changes the agent's capabilities based on a keyword heuristic, so ordinary coding requests such as reading a file or checking a branch can silently become ungrounded chat.

## Chosen Approach

Use one host-owned execution pipeline:

```text
ModelInput
  -> schema validation
  -> server-derived permission request
  -> permission decision
  -> exact invocation grant lookup
  -> execution
  -> typed outcome
  -> session/tool-result projection
```

Provider input contains only operation data. Authorization metadata never appears in a provider tool schema. When policy requires a user decision, deepcli persists an approval request containing the tool name, a redacted input summary, and a SHA-256 digest of the canonical tool name and arguments. The approval request UUID is the user-facing nonce. An approved request is a single-use grant and is consumable only by the exact tool name and argument digest that created it.

Repeated attempts for the same pending invocation reuse the existing request. High-risk requests need one confirmation; double-confirm risks need two distinct approval actions. The first double-confirm action leaves the request pending, and the second makes the grant ready. Consuming a grant changes its status before the side effect, providing at-most-once behavior if the process crashes.

Legacy approval records without an invocation digest remain readable but cannot authorize execution. The user must trigger the operation again to create a bound request.

## Permission Derivation

The host derives permission inputs from the actual operation:

- File tools resolve paths through the canonical workspace or the canonical nearest existing ancestor for new paths. A symlink that escapes the workspace is rejected.
- Direct file reads and writes apply `DeepIgnore`; provider tools cannot access ignored credentials, sessions, environment files, keys, or project-specific ignored paths.
- Shell safety flags are not accepted from the model. Fixed Git commands and file tools retain their dedicated surfaces. Generic shell commands are classified conservatively; destructive syntax remains double-confirm, safe test/build commands may receive deterministic automated review, and unknown or compound shell commands require explicit approval.
- `run_tests` accepts only a discovered test command plus shell-control-free argument extensions. It cannot act as a generic shell alias.
- Child processes remove provider credential environment variables before execution.

The existing `autoReviewer` option becomes a deterministic reviewer for recognized, shell-control-free `run_tests` invocations only and defaults to disabled. It does not auto-approve generic shell, network, or process operations and is not an OS isolation boundary.

## Outcome Semantics

`ToolExecution` owns a boolean success outcome in addition to structured data. Exit code zero is success for shell/Git commands; `run_tests` uses its `passed` result; timeout and non-zero exit are failures. The same outcome drives:

- provider tool-result `ok`;
- session lifecycle `Succeeded` or `Failed`;
- native terminal `ToolCompleted.ok`;
- runtime tool-budget failure accounting.

Rust-level execution errors remain errors. A completed process with a non-zero exit is a completed tool execution with `success = false`, preserving stdout/stderr for diagnosis without claiming success.

## Runtime Behavior

All normal prompts use the tool-capable agent loop. Streaming is a transport/presentation concern and never removes tools.

When a tool needs approval, the runtime records exactly one failed tool result for the current call, persists the invocation-bound request, and pauses the turn immediately in `AwaitingApproval`. Pending approval is a non-overridable completion blocker. A later exact retry consumes an approved grant; a changed argument set creates a new request.

This round does not add full in-flight keyboard cancellation or automatic continuation after approval. Those require a shared turn supervisor and parent/child cancellation propagation through provider requests and subprocess groups; they remain the next runtime/UI priority rather than being approximated in the permission layer.

Adjacent high-risk boundaries use the same fail-closed approach: `web_fetch` pins public DNS addresses across redirects and reads a host-bounded response body; `git_commit` binds approval to the staged tree and creates that exact tree with `commit-tree` plus an old-HEAD compare-and-swap update, without repository hooks; sub-agent allowed-tools and canonical read/write scopes are runtime capabilities rather than prompt hints, and nested capabilities can only narrow. Layered configuration is recursively merged as raw JSON before typed deserialization.

## Compatibility And Risk

- Provider tool schemas remove untrusted fields. This is an intentional internal contract break.
- Approval JSON gains invocation summary/digest and confirmation metadata. Existing status values remain readable; a new `consumed` status identifies a used grant.
- Old unbound approvals cannot be consumed.
- Shell and file access become more conservative. Calls previously allowed through self-declared flags may now require a real approval.
- No dependency or lockfile change is required.

## Verification

Focused tests must cover model self-approval rejection, exact-match and single-use grants, double confirmation, approval deduplication, malicious `run_tests` input, symlink escape, ignored-file access, subprocess credential scrubbing, failed tool outcome propagation, immediate approval pause, and tool availability for short prompts. Final verification includes fmt, all tests, clippy, build, native terminal smoke, quick preflight, privacy scan, and `git diff --check`.
