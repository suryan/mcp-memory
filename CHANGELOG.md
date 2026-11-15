# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-16

### Added

- Shared store process (`mcp-memory daemon`): one process owns the Helix graph; every harness and the CLI connect over a Unix socket under `run/` and start the process when it is not running.
- Concurrent harness support: several agents can write and recall against the same store at once; writes are serialised so duplicate detection holds.
- `mcp-memory import-kiro`: import every memory from Kiro's local SQLite database (`~/.kiro/memories/memories.db`, or `$KIRO_HOME/...`, or `--path`).
- Relation kind names in the `memory_remember` schema, server instructions, and rejection messages (`REQUIRES`, `PART_OF`, `RUNS_WITH`, `PREFERS`, `CORRECTS`, `RULED_OUT_FOR`).
- Root `README.md` and MIT `LICENSE`.
- Strict crate lints (clippy `all` / `pedantic`, no `unwrap` / `expect` / `panic` outside tests, `missing_docs`) and a clippy step in `scripts/precommit.sh`.
- `Draft` input for `Service::remember` instead of a long argument list.

### Changed

- Agents must pass `project_path` for repository memories. Without it, recall covers only global memories and writes are stored as global (with a note in the result). The stdio front end no longer sends its working directory.
- Content size limit raised from 4 KiB to **20 KiB**.
- License changed from Apache-2.0 to **MIT**.
- Diagram rendering pins Playwright's browser cache to `~/.cache/ms-playwright` so sandboxed shells do not download Chromium onto a full `/tmp`.
- Docs updated for the daemon layout, backup/restore, multi-harness use, and Kiro import.

### Fixed

- Compiler warnings in vendored `third_party/` crates used to clear the osv-scanner gate.

## [0.1.0] - 2026-11-15

### Added

- Initial local MCP memory server backed by embedded HelixDB v3.7.0.
- MCP tools: `memory_recall`, `memory_remember`, `memory_forget`, `memory_list`, `memory_graph`, and the `memory://digest` resource.
- Human CLI: `health`, `list`, `show`, `search`, `delete`, `export`, `import`, `config`.
- Access modes: `off`, `read_only`, `read_write`, `learn`.
- Local `token-hash-v1` embeddings (384 dimensions), BM25, and graph-hop recall fused in one Helix read.
- Patched transitive crates under `third_party/` so the osv-scanner CVE gate stays clean.

[0.2.0]: https://github.com/suryan/mcp-memory/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/suryan/mcp-memory/releases/tag/v0.1.0
