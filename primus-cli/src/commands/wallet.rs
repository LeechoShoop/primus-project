use anyhow::{Context, Result};
use primus_sdk::Wallet;
use std::path::Path;

pub fn create(path: String) -> Result<()> {
    println!("⚙️ Generating new ML-DSA-87 wallet...");

    let wallet = std::thread::Builder::new()
        .name("cli-keygen".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || Wallet::generate(24, 0))
        .context("Failed to spawn keygen thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("Keygen thread panicked"))?
        .context("Wallet generation failed")?;

    wallet.save(Path::new(&path))?;

    println!("✅ Created: {}", path);
    println!("🌐 Address:  {}", wallet.address);
    println!("🔑 Mnemonic: {}", wallet.get_mnemonic());
    println!("\n⚠️  IMPORTANT: Write down the mnemonic and store it offline.");

    Ok(())
}

pub fn show(path: String) -> Result<()> {
    let path_for_thread = path.clone(); // Клонируем для потока

    let wallet = std::thread::Builder::new()
        .name("cli-load".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            Wallet::load(Path::new(&path_for_thread)) // Используем клон
        })
        .context("Failed to spawn wallet-load thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("Load thread panicked"))?
        .context(format!("Failed to load wallet at {}", path))?; // Оригинал доступен здесь

    println!("📁 File:    {}", path);
    println!("🌐 Address: {}", wallet.address);

    Ok(())
}
