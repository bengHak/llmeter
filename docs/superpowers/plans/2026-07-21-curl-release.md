# Curl Install + GitHub Releases Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Approach A release packaging so users install llmeter with `curl … | sh` from GitHub Releases (macOS arm64 + Linux x86_64/arm64), defaulting to `~/.local/bin`, while keeping `cargo build --release` as the developer path.

**Architecture:** Tag-triggered GitHub Actions builds three target triples, packages root-level `llmeter` tarballs plus a shared `SHA256SUMS`, and publishes one Release per `v*` tag. Repo-hosted `scripts/install.sh` detects OS/arch, downloads and verifies the matching asset, and installs under `$HOME/.local/bin` (overridable).

**Tech Stack:** GitHub Actions, Rust release profile already in `Cargo.toml`, POSIX `sh` + `curl`/`wget` + `tar` + `sha256sum`/`shasum`; no new Rust crate dependencies.

**Design doc:** `docs/superpowers/specs/2026-07-21-curl-release-design.md`

## Global Constraints

- Do not implement Windows install or Windows binaries.
- Do not add macOS notarization, code signing, Homebrew, cargo-binstall, or cargo-dist.
- Do not change llmeter TUI/runtime behavior.
- Default install path is `$HOME/.local/bin`; never require `sudo` by default.
- Install script is served from `master` raw URL; binaries come only from GitHub Releases.
- Partial multi-target uploads are forbidden: publish only after all matrix builds succeed.
- Artifact names: `llmeter-<version>-<target>.tar.gz` with version **without** leading `v`.
- `Cargo.toml` version must match the tag version (tag has leading `v`).

---

### Task 1: Release workflow skeleton and version gate

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Trigger: `push` tags `v*`
- Permissions: `contents: write`
- Extract version from `GITHUB_REF_NAME` (strip leading `v`)
- Fail if `Cargo.toml` `[package].version` ≠ extracted version

- [ ] Add `release.yml` with tag trigger and `contents: write`.
- [ ] Add a prepare step that parses the tag and greps/awks `Cargo.toml` version; fail on mismatch.
- [ ] Document expected tag format (`v0.1.0`) in workflow comments.
- [ ] Dry-run logic locally with shell snippets (no need to push tags yet).

### Task 2: Matrix builds and packaging

**Files:**
- Modify: `.github/workflows/release.yml`

**Targets:**
- `aarch64-apple-darwin` on `macos-14`
- `x86_64-unknown-linux-gnu` on `ubuntu-latest`
- `aarch64-unknown-linux-gnu` on `ubuntu-latest` via `cross` (or equivalent)

**Packaging rules:**
- Build: `cargo build --release --target <triple>`
- Tarball contains only `./llmeter` at archive root
- Name: `llmeter-${VERSION}-${TARGET}.tar.gz`
- Upload per-job artifacts for the publish job

- [ ] Define the three-target matrix with correct runners.
- [ ] Install toolchain / target / cross as needed; honor `rust-toolchain.toml`.
- [ ] Package tarball with executable bit preserved.
- [ ] Upload build artifacts named consistently for the publish job.

### Task 3: SHA256SUMS and Release publish

**Files:**
- Modify: `.github/workflows/release.yml`

**Interfaces:**
- Publish job `needs` all build jobs
- Produce `SHA256SUMS` with two-space `sha256sum` lines for every tarball
- Create/update GitHub Release for the tag and attach all tarballs + `SHA256SUMS`
- Prefer `softprops/action-gh-release` (or equivalent) with `GITHUB_TOKEN`

- [ ] Implement publish job that downloads all matrix artifacts.
- [ ] Generate single `SHA256SUMS` covering all three archives.
- [ ] Attach assets only when every target succeeded.
- [ ] Set release name to `llmeter <version>`.

### Task 4: install.sh

**Files:**
- Create: `scripts/install.sh`

**Behavior (must match design):**
- `set -euo pipefail` (or safe POSIX equivalent)
- Detect Darwin/Linux and aarch64/x86_64; map to the three supported triples
- Reject Darwin x86_64 and other OS/arch with a clear error
- `LLMETER_VERSION` optional; default = GitHub `releases/latest` tag
- Accept `v0.1.0` or `0.1.0`; normalize to no leading `v` for filenames, with leading `v` for download URL tag path
- `INSTALL_DIR` default `$HOME/.local/bin`; `LLMETER_REPO` default `bengHak/llmeter`
- Download tarball + `SHA256SUMS` over HTTPS; verify hash; extract; install; `chmod +x`
- Print PATH hint if `INSTALL_DIR` not on `PATH`
- Prefer `curl`, fall back to `wget` if needed
- No `sudo`

- [ ] Implement OS/arch detection and target mapping.
- [ ] Implement latest + pinned version resolution via GitHub API/redirects.
- [ ] Implement download, checksum verify, extract, install.
- [ ] Implement PATH and version success messaging; stderr for errors.
- [ ] Manually exercise with a fake local asset layout or mock server if no real release exists yet (optional offline unit: shellcheck / dry functions).

### Task 5: install.sh structural tests / smoke harness

**Files:**
- Create: `scripts/test-install-sh.sh` (or `tests/install_script/` helper)
- Optional: `.github/workflows/ci.yml` step only if a general CI already exists; otherwise keep as local script

**Purpose:** Prove the **shipped** `scripts/install.sh` behavior without publishing a real release when possible.

- [ ] Add a test harness that sources or runs install.sh against a local HTTP fixture directory serving tarball + SHA256SUMS (or monkeypatches download URLs via `LLMETER_*` / temp overrides if added for testability).
- [ ] Assert: good checksum installs binary to temp `INSTALL_DIR`.
- [ ] Assert: bad checksum exits non-zero and does not leave a bad binary in place.
- [ ] Assert: unsupported arch path exits non-zero (simulate via env override if required).
- [ ] If full HTTP fixture is too heavy for v1, at minimum: `sh -n scripts/install.sh` syntax check + a pure function unit file extracted for version/triple helpers — prefer fixture path when feasible.

**Note:** Prefer testing real install.sh entrypoint with fixture server over re-implementing detection logic in the test.

### Task 6: README install UX

**Files:**
- Modify: `README.md` section 「2. 설치와 실행」

**Content order:**
1. Recommended: `curl -fsSL https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh | sh`
2. Pin: `LLMETER_VERSION=0.1.0` example
3. Default path `~/.local/bin` and PATH note
4. Supported platforms list (macOS arm64, Linux x86_64, Linux arm64)
5. Developer path: Rust 1.85+, `cargo build --release`, `./target/release/llmeter`
6. Optional: review-before-pipe pattern (`curl -o install.sh && less install.sh && sh install.sh`)
7. Known limit: no Windows; no notarization (quarantine possible on macOS)

- [ ] Rewrite install section per content order above.
- [ ] Keep subsequent usage examples unchanged unless paths break.
- [ ] Link or point to GitHub Releases for manual downloaders.

### Task 7: End-to-end verification checklist (implementer)

**Files:** none required beyond prior tasks

- [ ] `sh -n scripts/install.sh` passes.
- [ ] Fixture/harness tests for install.sh pass when present.
- [ ] Workflow YAML validates structurally (required keys, three targets, publish needs builds).
- [ ] Grep README for curl install one-liner and `~/.local/bin`.
- [ ] Grep design + this plan for non-goals: Windows, notarization, cargo-dist excluded.
- [ ] `git diff --check` on changed files.
- [ ] Do **not** push a real production tag as part of this plan unless the owner explicitly requests a first release; first `v*` publish is a separate ops step after merge.

---

## Suggested PR split

1. **PR1:** `scripts/install.sh` + harness + README install section (docs-usable even before first tag if message handles empty releases).
2. **PR2:** `.github/workflows/release.yml` packaging + publish.
3. **Ops (manual):** bump version if needed, tag `vX.Y.Z`, verify Release assets, run curl|sh on a clean machine.

## Out of scope (do not implement in these tasks)

- Windows binaries or PowerShell install
- Apple signing/notarization
- Homebrew / cargo-dist / cargo-binstall
- musl static builds
- Automatic updater daemon
- Changing application source under `src/`
