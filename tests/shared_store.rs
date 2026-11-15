//! Several harness processes and the CLI share one store through the store process.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "helpers drive real processes; any unexpected state fails the test"
)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_mcp-memory");

fn command(home: &Path) -> Command {
    let mut command = Command::new(BIN);
    command
        .env("MCP_MEMORY_HOME", home)
        .env("MCP_MEMORY_IDLE_SECS", "2")
        .env("HELIX_TELEMETRY_LEVEL", "off")
        .env("RUST_LOG", "warn");
    command
}

struct Agent {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Agent {
    fn spawn(home: &Path, cwd: &Path) -> Self {
        let mut child = command(home)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("bad reply {line:?}"));
        assert_eq!(reply["id"], id, "reply out of order: {reply}");
        reply
    }

    fn initialize(&mut self) {
        let reply = self.call(
            "initialize",
            &json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"},
            }),
        );
        assert_eq!(reply["result"]["serverInfo"]["name"], "mcp-memory", "{reply}");
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }

    fn tool(&mut self, name: &str, arguments: &Value) -> Value {
        let reply = self.call("tools/call", &json!({"name": name, "arguments": arguments}));
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no tool text: {reply}"));
        serde_json::from_str(text).unwrap()
    }

    fn remember(&mut self, title: &str) -> Value {
        self.tool(
            "memory_remember",
            &json!({
                "title": title,
                "summary": title,
                "content": format!("{title}."),
                "type": "preference",
                "scope": "global",
                "explicit": true,
            }),
        )
    }

    fn recall_titles(&mut self, query: &str) -> Vec<String> {
        let reply = self.tool("memory_recall", &json!({"query": query}));
        assert_eq!(reply["action"], "recalled", "{reply}");
        reply["titles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|title| title.as_str().unwrap().to_string())
            .collect()
    }

    fn close(mut self) {
        drop(self.stdin);
        let status = self.child.wait().unwrap();
        assert!(status.success(), "agent exited with {status}");
    }
}

fn cli(home: &Path, args: &[&str]) -> Value {
    let output = command(home).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn titles(listed: &Value) -> Vec<String> {
    let mut titles: Vec<String> = listed["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|memory| memory["title"].as_str().unwrap().to_string())
        .collect();
    titles.sort();
    titles
}

fn store_pid(home: &Path) -> u32 {
    std::fs::read_to_string(home.join("run/daemon.lock"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// A zombie still has a /proc entry until its new parent reaps it.
fn alive(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(") ")
            .is_some_and(|(_, rest)| !rest.starts_with('Z')),
        Err(_) => false,
    }
}

#[test]
fn two_agents_and_the_cli_share_one_store() {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();

    // Both start before any store process exists, so they race to start one.
    let mut a = Agent::spawn(home.path(), repo.path());
    let mut b = Agent::spawn(home.path(), repo.path());
    a.initialize();
    b.initialize();

    assert_eq!(a.remember("Prefer tabs in Makefiles")["action"], "created");
    assert_eq!(b.remember("Use pnpm for node projects")["action"], "created");
    assert_eq!(a.remember("Run clippy before every commit")["action"], "created");

    assert!(a.recall_titles("pnpm node projects").contains(&"Use pnpm for node projects".to_string()));
    assert!(b.recall_titles("tabs Makefiles").contains(&"Prefer tabs in Makefiles".to_string()));

    let listed = cli(home.path(), &["list"]);
    assert_eq!(
        titles(&listed),
        vec![
            "Prefer tabs in Makefiles",
            "Run clippy before every commit",
            "Use pnpm for node projects",
        ]
    );
    assert_eq!(cli(home.path(), &["health"])["helix"], "ok");

    // Agents still work after the CLI used the store.
    assert!(b.recall_titles("clippy commit").contains(&"Run clippy before every commit".to_string()));

    let pid = store_pid(home.path());
    a.close();
    b.close();

    let deadline = Instant::now() + Duration::from_secs(20);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!alive(pid), "store process stayed up while idle");
    assert!(!home.path().join("run/mcp-memory.sock").exists());

    // The next command starts a fresh store process over the same data.
    assert_eq!(titles(&cli(home.path(), &["list"])).len(), 3);
}
