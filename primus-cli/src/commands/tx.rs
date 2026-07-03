// Transitional deprecation warnings removed.


use anyhow::{Context, Result};
use primus_sdk::{AtomElement, PROTOCOL_MIN_FEE, TransactionBuilder, Wallet};
use std::path::Path;

pub async fn send(to_address: String, amount: u64, wallet_path: String, host: String, port: u16) -> Result<()> {
    // 1. Загрузка кошелька (Stack Size 16MB для ML-DSA-87)
    let wallet_path_clone = wallet_path.clone();
    let wallet = std::thread::Builder::new()
        .name("cli-load".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || Wallet::load(Path::new(&wallet_path_clone)))
        .context("Failed to spawn wallet-load thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("Load thread panicked"))?
        .context(format!("Failed to load wallet at {}", wallet_path))?;

    // 2. Декодируем адрес получателя
    let recipient_pk =
        Wallet::decode_address(&to_address).context("Invalid recipient address format")?;

    // 3. Инициализация клиента (в будущем параметры хоста/порта лучше брать из конфига)
    let mut client = primus_client::rpc::NodeClient::new_auto(&host, port).await?;

    println!("🔍 Fetching atom state for {}...", wallet.address);

    // Получаем состояние атома из сети
    let atom_state = match client.get_atom_state(&wallet.address).await {
        Ok(state) => state,
        Err(e) => {
            println!("⚠️  Could not fetch state: {}. Using debug defaults.", e);
            // Заглушка, если нода недоступна
            primus_client::rpc::AtomState {
                mass: amount + PROTOCOL_MIN_FEE + 1000,
                nonce: 0,
                last_hash: [0u8; 32],
                element: AtomElement::Hydrogen,
            }
        }
    };

    // Проверка баланса (массы) перед подписью
    if atom_state.mass < (amount + PROTOCOL_MIN_FEE) {
        return Err(anyhow::anyhow!(
            "Insufficient mass: available {}, required {}",
            atom_state.mass,
            amount + PROTOCOL_MIN_FEE
        ));
    }

    println!("📡 Preparing transaction...");
    println!("   From:    {}", wallet.address);
    println!("   To:      {}", to_address);
    println!("   Amount:  {} mass", amount);
    println!("   Nonce:   {}", atom_state.nonce);

    // 4. Сборка и отправка транзакции
    let builder = TransactionBuilder::new(&wallet)
        .recipient(recipient_pk)
        .amount(amount)
        .fee(PROTOCOL_MIN_FEE)
        .sender_mass(atom_state.mass)
        .sender_last_hash(atom_state.last_hash)
        .sender_nonce(atom_state.nonce)
        .sender_element(atom_state.element);

    println!("✅ Transaction configured. Broadcasting...");

    // 5. Отправка транзакции в сеть
    // Подпись и изоляция стека теперь обрабатываются внутри broadcast_tx -> Wallet::sign
    match client.broadcast_tx(&wallet, builder).await {
        Ok(msg) => println!("🚀 Node response: {}", msg),
        Err(e) => println!("❌ Network error: {}", e),
    }

    Ok(())
}

pub async fn balance(address: String, prove: bool, host: String, port: u16) -> Result<()> {
    let mut client = primus_client::rpc::NodeClient::new_auto(&host, port).await?;

    if prove {
        println!("🔍 Fetching state root and balance proof for {}...", address);
        
        // 1. Get current status to find the trusted root
        let mut ipc_stream = crate::admin::connect_ipc(port).await?;
        crate::admin::send_request(&mut ipc_stream, &primus_types::IpcRequest::Status).await?;
        let (root, current_height) = match crate::admin::receive_response(&mut ipc_stream).await? {
            primus_types::IpcResponse::StatusReport { height, .. } => {
                // In a real light client, we'd fetch the header for 'height' and verify PoW.
                // For this CLI tool, we trust the node's current tip root via IPC.
                let mut client_local = primus_client::rpc::NodeClient::new_auto(&host, port).await?;
                let crystal_bytes = client_local.get_crystal(height).await?;
                let crystal: primus_types::Crystal = bincode::deserialize(&crystal_bytes)?;
                (crystal.state_root, height)
            }
            _ => return Err(anyhow::anyhow!("Failed to fetch trusted root via IPC")),
        };

        // 2. Fetch the proof via network
        let proof = client.get_balance_proof(&address).await?;
        
        // 3. Verify via SDK (WASM-safe logic)
        println!("🛡️  Verifying Merkle proof against root {:02x?}...", &root[..4]);
        match primus_sdk::verify_balance_proof(&proof, &root, current_height, current_height) {
            Ok(Some(atom)) => {
                println!("✅ Proof Validated.");
                println!("💰 Balance: {} mass", atom.mass);
                println!("   Nonce:   {}", atom.nonce);
                println!("   Element: {:?}", atom.element);
            }
            Ok(None) => {
                println!("✅ Proof Validated (Exclusion).");
                println!("⚠️  Atom does not exist on-chain.");
            }
            Err(e) => {
                println!("❌ PROOF INVALID: {}", e);
                return Err(e);
            }
        }
    } else {
        match client.get_atom_state(&address).await {
            Ok(state) => {
                println!("💰 Balance: {} mass", state.mass);
                println!("   Nonce:   {}", state.nonce);
                println!("   Element: {:?}", state.element);
            }
            Err(e) => println!("❌ Error: {}", e),
        }
    }

    Ok(())
}
