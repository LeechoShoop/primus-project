// primus-cli/src/config.rs
//
// Node configuration helpers for the Primus CLI.

/// Default IPC endpoint used by the CLI to talk to the running node.
#[cfg(unix)]
pub const DEFAULT_IPC_PATH: &str = "/tmp/primus.sock";

#[cfg(windows)]
#[allow(dead_code)]
pub const DEFAULT_IPC_PATH: &str = "primus-pipe";

/// Default node RPC / TCP port.
#[allow(dead_code)]
pub const DEFAULT_NODE_PORT: u16 = 9000;
