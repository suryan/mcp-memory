# Usage

`mcp-memory` is a local MCP server. A coding harness starts the binary with no arguments and talks to it over stdio. Statements are stored in an embedded HelixDB graph on disk, one personal space per machine.

## Install

### One-liner (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/suryan/mcp-memory/main/scripts/install.sh \
  | bash -s -- --yes
```

That installs `~/.local/bin/mcp-memory` (bootstraps rustup if needed). See `scripts/install.sh --help` for `--ref`, `--prefix`, and `--local`.

### From this repo

With Rust 1.99.1 (pinned in `rust-toolchain.toml`):

```bash
./scripts/install.sh --local
# or:
cargo build --release
install -m 0755 target/release/mcp-memory ~/.local/bin/mcp-memory
```

The binary is `target/release/mcp-memory`. Put that path in the harness config, or install it onto `PATH` and use the name `mcp-memory`.

## Connect a harness

```json
{
  "mcpServers": {
    "memory": {
      "command": "mcp-memory"
    }
  }
}
```

No arguments. The process serves MCP until the harness closes stdin. If `HELIX_TELEMETRY_LEVEL` is unset, it is set to `off` before the graph opens.

The server sends its own instructions. For a client that drops them, the same policy is in [harness-snippet.md](harness-snippet.md).

## Several harnesses at once

Cursor, Claude Code, Codex, and the CLI can all use the store at the same time. Each harness's `mcp-memory` process forwards to one shared store process, which is the only process that opens the graph. The first harness or CLI command starts it. It exits five minutes after the last connection closes; set `MCP_MEMORY_IDLE_SECS` to change that. Its log is `daemon.log` in the data directory.

After rebuilding the binary, the new version is used once the old store process has exited.

## Access

Default mode is `learn`. Change it from the CLI. The human CLI can still list, show, and delete when the agent cannot write.

| Mode | Agent |
|---|---|
| `off` | No recall and no writes |
| `read_only` | Recall and list only |
| `read_write` | Recall, plus writes the user asked for (`explicit: true`) |
| `learn` | Recall, explicit writes, and automatic writes |

```bash
mcp-memory config show
mcp-memory config set read_only
```

## What to store

One statement: a title, a one-line summary, and the content. Types are `preference`, `fact`, `convention`, `correction`, and `ruled_out`. Scope is `global` (how you want work done) or `repository` (true of this repo).

Leave out chat transcripts, file bodies, one-off task instructions, anything already in project rules, and secrets. Secret-shaped text is rejected and is not written to the log. Project rules win when they disagree with a memory.

## Agent tools

- `memory_recall` — task wording in, relevant active memories out. An unrelated query comes back empty.
- `memory_remember` — create, update, or supersede one memory. Set `explicit` to true only when you asked for the write. Pass `supersedes` with the old id when a new statement replaces one.
- `memory_forget` — delete one memory by id. There is no delete-by-query.
- `memory_list` — browse what is stored. This is not task recall.
- `memory_graph` — entities linked to a name, and the memories that cite them.
- `memory://digest` — titles and summaries of global memories.

The agent passes `project_path`, the absolute path of the repository it is working in, to `memory_recall` and `memory_remember`. The server does not guess a repository: harnesses start it in different directories (Cursor uses your home directory), and the store process runs in the data directory. With a path, recall returns global memories plus that repository, and nothing from a different repo. Without one, recall returns only global memories, and a write is stored as global even when it asks for `scope: repository`; the result then says `"scope": "global"` with a note. The server instructions tell agents to pass the path.

## Human CLI

```bash
mcp-memory health
mcp-memory list
mcp-memory list --scope global --type preference
mcp-memory show <id>
mcp-memory search "integration tests"
mcp-memory search "seed the database" --project-path /path/to/repo
mcp-memory delete <id>
mcp-memory export memories.json
mcp-memory import memories.json
mcp-memory import-kiro
mcp-memory import-kiro --path /path/to/memories.db
```

`import-kiro` reads every row from Kiro's SQLite database (default `~/.kiro/memories/memories.db`, or `$KIRO_HOME/memories/memories.db`). It keeps Kiro ids, maps `USER_PREFERENCE` to `preference` and other types to `fact`, and maps `Global` / `repo:…` scopes onto ours. Re-running is safe: an unchanged id is skipped. Kiro repo keys look like `github/owner/repo` and may need a `repo_overrides` pin to match a local checkout. Content may be up to 20 KiB.

`export` writes a JSON list of statements. Embeddings are omitted and rebuilt on `import`. Entity edges are omitted. Importing the same id with different content marks that memory `conflicted` and leaves it out of recall. A full copy of the store is the data directory below.

## Where the data is

`mcp-memory config show` prints the directory. The default is `~/.local/share/mcp-memory`. `MCP_MEMORY_HOME` replaces that path. When `XDG_DATA_HOME` is set, the default is `$XDG_DATA_HOME/mcp-memory`.

That directory is the store:

- `config.toml` holds the access mode and the `data_dir` path.
- `helix/` is the graph: memories, entities, edges, and embeddings.
- `run/` holds the store process's socket and lock while it runs.
- `daemon.log` is the store process's log.

Stopping the process leaves the directory on disk.

## Backup and restore

Close every harness and wait for the store process to exit (no `run/mcp-memory.sock`), then copy the whole directory. That copy is the backup for an OS wipe or a new laptop. Put it back at the same path. If the home directory path changed, set `data_dir` in `config.toml` to the restored folder. The program reads `config.toml` from the default directory, then opens the graph at the `data_dir` inside that file.

A repository memory keyed by a git remote still matches after the checkout moves. A repository with no remote keeps its old absolute path as the key.

## Limits of this build

Recall uses keyword, graph links, and a local `token-hash-v1` embedding (384 dimensions). Shared wording ranks closer than unrelated text. This is not paraphrase recall. `mcp-memory health` reports the model id. There is no admin HTTP API and no shared team server in this build. Contributor setup is in [development.md](development.md).
