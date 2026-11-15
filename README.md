# mcp-memory

Local MCP memory server for coding agents. One process owns an embedded HelixDB knowledge graph; each harness talks to it over stdio and shares the same store with every other harness and the CLI.

Agents store durable statements (preferences, facts, conventions, corrections, ruled-out paths) and recall them by keyword, graph links, and a local embedding. The store lives on disk under `~/.local/share/mcp-memory` by default.

## Install

Rust 1.99.1 is pinned in `rust-toolchain.toml`. From this repo:

```bash
cargo build --release
install -m 0755 target/release/mcp-memory ~/.local/bin/mcp-memory
```

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

No arguments. The binary forwards stdio to a shared store process (`mcp-memory daemon`), starting it when none is running. Several harnesses and the CLI can use the store at once.

## Agent tools

| Tool | Purpose |
|---|---|
| `memory_recall` | Relevant active memories for the current task |
| `memory_remember` | Create, update, or supersede one memory |
| `memory_forget` | Delete one memory by id |
| `memory_list` | Browse what is stored |
| `memory_graph` | Entities linked to a name, and memories that cite them |
| `memory://digest` | Titles and summaries of global memories |

Pass `project_path` (absolute path of the repository in play) to `memory_recall` and `memory_remember`. Without it, recall covers only global memories and writes are stored as global.

## CLI

```bash
mcp-memory health
mcp-memory list
mcp-memory search "integration tests"
mcp-memory show <id>
mcp-memory delete <id>
mcp-memory export memories.json
mcp-memory import memories.json
mcp-memory config show
mcp-memory config set read_only
```

## Docs

| Doc | Contents |
|---|---|
| [docs/usage.md](docs/usage.md) | Install, harness config, access modes, backup |
| [docs/architecture.md](docs/architecture.md) | Components, graph shape, decisions |
| [docs/development.md](docs/development.md) | Toolchain, precommit gate, contributor setup |
| [docs/prd.md](docs/prd.md) | Product requirements |

```bash
bash scripts/precommit.sh
```

## License

MIT. See [LICENSE](LICENSE).
