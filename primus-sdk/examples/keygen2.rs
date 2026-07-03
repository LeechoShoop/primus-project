// =============================================================================
// examples/keygen.rs — Operator Wallet Generator for Primus-Project
//
// USAGE:
//   cargo run --example keygen
//
// WHAT IT DOES:
//   1. Creates the `.secrets/` directory if it does not exist.
//   2. Generates a fresh 24-word BIP-39 wallet at derivation index 1
//      (Operator role) using the primus-sdk Wallet API.
//   3. Saves the wallet to `.secrets/operator.wallet` via Wallet::save().
//      The file stores ONLY the mnemonic + index — never raw key bytes.
//   4. Prints the full public address (hex) and the mnemonic backup phrase.
//
// SECURITY NOTES:
//   • The mnemonic printed to stdout is the ROOT SECRET for this operator key.
//     Write it down on paper and store it somewhere safe offline.
//   • After this tool exits, set file permissions:
//       Unix:    chmod 600 .secrets/operator.wallet
//       Windows: use File Properties → Security to restrict access.
//   • If `.secrets/operator.wallet` already exists this tool aborts safely —
//     it will never silently overwrite an existing key.
//
// STACK:
//   ML-DSA-87 key generation requires ~4 MiB of stack. The main thread on
//   Windows defaults to 1 MiB, so all keygen work runs inside a dedicated
//   thread with an explicit 16 MiB stack.
// =============================================================================

use primus_sdk::Wallet;
use std::path::Path;

fn main() {
    // All output goes to stdout so it can be piped / captured by scripts.
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  🔑  Primus-Project — Operator Wallet Generator");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  🚨 WARNING: THIS IS A DETERMINISTICALLY SEEDED KEY GENERATOR (FOR TESTING).");
    println!("  🚨 DO NOT USE THIS FOR REAL FUNDS ON MAINNET!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // ── 1. Ensure .secrets/ exists ────────────────────────────────────────────
    let secrets_dir = Path::new(".secrets");
    let wallet_file_path = secrets_dir.join("operator.wallet");

    if !secrets_dir.exists() {
        std::fs::create_dir_all(secrets_dir).expect("❌ Failed to create .secrets/ directory");
        println!("📁 Created directory: .secrets/");
    }

    // ── 2. Abort if a wallet already exists — never overwrite silently ────────
    if wallet_file_path.exists() {
        println!();
        println!("⚠️  .secrets/operator.wallet already exists.");
        println!("    Delete it manually if you really want to generate a new key.");
        println!("    Aborting — your existing key has NOT been modified.");
        println!();
        std::process::exit(0);
    }

    // ── 3. Key generation inside a 16 MiB stack thread (Windows-safe) ────────
    //
    // ML-DSA-87 key expansion allocates ~4 MiB on the stack. The default main-
    // thread stack on Windows is 1 MiB; spawning an explicit thread with a
    // generous 16 MiB budget prevents a stack overflow on all platforms.
    println!();
    println!("⚙️  Generating 24-word ML-DSA-87 wallet (index 1 = Operator)...");
    println!("    This may take a moment — post-quantum key expansion is intensive.");

    let wallet = std::thread::Builder::new()
        .name("keygen".into())
        .stack_size(16 * 1024 * 1024) // 16 MiB — safe on Windows and Linux
        .spawn(|| {
            // index = 1 → Operator Key role (per the SDK derivation table)
            Wallet::generate(24, 1)
                .expect("❌ Wallet generation failed — check primus-sdk is correctly linked")
        })
        .expect("❌ Failed to spawn keygen thread")
        .join()
        .expect("❌ Keygen thread panicked");

    // ── 4. Persist to .secrets/operator.wallet ────────────────────────────────
    wallet
        .save(&wallet_file_path)
        .expect("❌ Failed to save wallet file");

    // ── 5. Print results ──────────────────────────────────────────────────────
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  ✅  Operator wallet generated successfully!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("  📄 File:    .secrets/operator.wallet");
    println!("  🔢 Index:   1 (Operator Key)");
    println!();
    println!("  🌐 Full address (share this publicly):");
    println!("  {}", wallet.address);
    println!();

    // The mnemonic is the root secret — display it with prominent warnings.
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  ⚠️  MNEMONIC BACKUP PHRASE — CRITICAL SECRET");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
    println!("  {}", wallet.get_mnemonic());
    println!();
    println!("  ▸ Write these 24 words down on paper — in order.");
    println!("  ▸ Store the paper somewhere physically secure and offline.");
    println!("  ▸ NEVER type them into any website, email, or chat.");
    println!("  ▸ Anyone who reads these words controls your Operator key.");
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Next step (Unix):    chmod 600 .secrets/operator.wallet");
    println!("  Start the node:      cargo run -- --operator-key .secrets/operator.wallet");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}
